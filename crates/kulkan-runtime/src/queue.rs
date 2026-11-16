use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use kulkan_ipc::{
    Message, PlayCountUpdate, PlaybackSnapshot, PlaybackStatus, QueueEntry, QueueSnapshot,
    RepeatMode,
};
use serde::{Deserialize, Serialize};

use crate::playback::PlaybackService;
use crate::playback_attempt::PlaybackAttempt;
use crate::{LibraryService, PersistedLibraryState};

const QUEUE_STATE_VERSION: u16 = 1;
const PORTABLE_STATE_VERSION: u16 = 1;
static NEXT_QUEUE_STATE_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub(super) struct QueueService {
    playback: PlaybackService,
    entries: Vec<QueueEntry>,
    current_entry_id: Option<u64>,
    next_entry_id: u64,
    repeat_mode: RepeatMode,
    shuffle: bool,
    last_failure: Option<String>,
    attempt: Option<PlaybackAttempt>,
    pending_play_count_updates: Vec<PlayCountUpdate>,
    state_path: Option<PathBuf>,
    dirty: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct PersistedQueueState {
    version: u16,
    entries: Vec<QueueEntry>,
    current_entry_id: Option<u64>,
    next_entry_id: u64,
    repeat_mode: RepeatMode,
    shuffle: bool,
}

#[derive(Deserialize, Serialize)]
struct PortableAppState {
    version: u16,
    library: PersistedLibraryState,
    queue: PersistedQueueState,
}

impl QueueService {
    pub(super) fn load(
        audio_sink: Option<String>,
        state_path: Option<PathBuf>,
        library: &mut LibraryService,
        start_paused: bool,
    ) -> io::Result<Self> {
        let persisted = match state_path.as_deref().map(fs::read) {
            Some(Ok(bytes)) => {
                let persisted: PersistedQueueState =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("could not read queue state: {error}"),
                        )
                    })?;
                if persisted.version != QUEUE_STATE_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported queue state version {}", persisted.version),
                    ));
                }
                validate_queue_state(&persisted)?;
                Some(persisted)
            }
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Some(Err(_)) | None => None,
        };
        let mut queue = Self {
            playback: PlaybackService::new(audio_sink),
            entries: persisted
                .as_ref()
                .map_or_else(Vec::new, |state| state.entries.clone()),
            current_entry_id: persisted.as_ref().and_then(|state| state.current_entry_id),
            next_entry_id: persisted
                .as_ref()
                .map_or(1, |state| state.next_entry_id.max(1)),
            repeat_mode: persisted
                .as_ref()
                .map_or(RepeatMode::Off, |state| state.repeat_mode),
            shuffle: persisted.as_ref().is_some_and(|state| state.shuffle),
            last_failure: None,
            attempt: None,
            pending_play_count_updates: Vec::new(),
            state_path,
            dirty: false,
        };
        if let Some(current_entry_id) = queue.current_entry_id {
            if queue
                .entries
                .iter()
                .any(|entry| entry.id == current_entry_id && !entry.failed)
                && let Err(error) =
                    queue.play_entry_with_mode(current_entry_id, library, start_paused)
            {
                return Err(io::Error::other(error));
            }
        }
        if queue.dirty {
            queue.persist_state()?;
        }
        Ok(queue)
    }

    pub(super) fn handles(message: &Message) -> bool {
        PlaybackService::handles(message)
            || matches!(
                message,
                Message::AppendQueue(_)
                    | Message::ReplaceQueue(_)
                    | Message::ReplaceFirstQueueEntry(_)
                    | Message::ListenNextQueueEntry(_)
                    | Message::GetQueueState
                    | Message::PlayQueueEntry(_)
                    | Message::PreviousQueueEntry
                    | Message::SkipQueueEntry
                    | Message::ReorderQueueEntry { .. }
                    | Message::RemoveQueueEntry(_)
                    | Message::ClearQueue
                    | Message::SetRepeatMode(_)
                    | Message::SetShuffle(_)
                    | Message::ExportAppState(_)
                    | Message::ImportAppState(_)
                    | Message::ResetAppState
            )
    }

    pub(super) fn handle(&mut self, message: Message, library: &mut LibraryService) -> Message {
        match message {
            Message::ExportAppState(path) => match self.export_app_state(&path, library) {
                Ok(()) => self.transfer_complete(library),
                Err(error) => Message::LibraryOperationFailed(error.to_string()),
            },
            Message::ImportAppState(path) => match self.import_app_state(&path, library) {
                Ok(()) => self.transfer_complete(library),
                Err(error) => Message::LibraryOperationFailed(error.to_string()),
            },
            Message::ResetAppState => match self.reset_app_state(library) {
                Ok(()) => self.transfer_complete(library),
                Err(error) => Message::LibraryOperationFailed(error),
            },
            Message::AppendQueue(tracks) => {
                self.append(tracks);
                self.queue_state_message(library)
            }
            Message::ReplaceQueue(tracks) => match self.replace_queue_and_play(tracks, library) {
                Ok(()) => self.queue_state_message(library),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::ReplaceFirstQueueEntry(track) => {
                match self.replace_first_and_play(track, library) {
                    Ok(()) => self.queue_state_message(library),
                    Err(error) => Message::QueueOperationFailed(error),
                }
            }
            Message::ListenNextQueueEntry(track) => match self.listen_next(track, library) {
                Ok(()) => self.queue_state_message(library),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::GetQueueState => self.queue_state_message(library),
            Message::PlayQueueEntry(entry_id) => match self.play_entry(entry_id, library) {
                Ok(()) => self.queue_state_message(library),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::PreviousQueueEntry => match self.previous(library) {
                Ok(()) => self.queue_state_message(library),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::SkipQueueEntry => {
                if let Err(error) = self.observe_current(library) {
                    return Message::QueueOperationFailed(error);
                }
                self.advance(AdvanceKind::Skipped, library, false);
                self.queue_state_message(library)
            }
            Message::ReorderQueueEntry { entry_id, position } => {
                match self.reorder_entry(entry_id, position) {
                    Ok(()) => self.queue_state_message(library),
                    Err(error) => Message::QueueOperationFailed(error),
                }
            }
            Message::RemoveQueueEntry(entry_id) => match self.remove_entry(entry_id, library) {
                Ok(()) => self.queue_state_message(library),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::ClearQueue => {
                if let Err(error) = self.observe_current(library) {
                    return Message::QueueOperationFailed(error);
                }
                self.entries.clear();
                self.current_entry_id = None;
                self.last_failure = None;
                self.attempt = None;
                self.dirty = true;
                let _ = self.playback.handle(Message::StopPlayback);
                self.queue_state_message(library)
            }
            Message::SetRepeatMode(repeat_mode) => {
                self.dirty |= self.repeat_mode != repeat_mode;
                self.repeat_mode = repeat_mode;
                self.queue_state_message(library)
            }
            Message::SetShuffle(enabled) => {
                self.dirty |= self.shuffle != enabled;
                if enabled && !self.shuffle {
                    let first_upcoming = self
                        .current_entry_id
                        .and_then(|id| self.entries.iter().position(|entry| entry.id == id))
                        .map_or(0, |index| index + 1);
                    self.shuffle_range(first_upcoming, self.entries.len());
                }
                self.shuffle = enabled;
                self.queue_state_message(library)
            }
            Message::PlayTrack(path) => self.play_track(path, library),
            Message::PausePlayback => self.change_playback(Message::PausePlayback, library),
            Message::ResumePlayback => self.change_playback(Message::ResumePlayback, library),
            Message::SeekPlayback(position_ms) => self.seek_playback(position_ms, library),
            Message::StopPlayback => self.stop_playback(library),
            Message::GetPlaybackState => match self.observe_current(library) {
                Ok(snapshot) => Message::PlaybackState(snapshot),
                Err(error) => Message::PlaybackOperationFailed(error),
            },
            _ => Message::QueueOperationFailed("unsupported queue operation".to_owned()),
        }
    }

    fn queue_state_message(&mut self, library: &mut LibraryService) -> Message {
        match self.snapshot(library) {
            Ok(snapshot) => match self.persist_state() {
                Ok(()) => Message::QueueState(snapshot),
                Err(error) => Message::QueueOperationFailed(error.to_string()),
            },
            Err(error) => Message::QueueOperationFailed(error),
        }
    }

    fn transfer_complete(&self, library: &LibraryService) -> Message {
        Message::AppStateTransferComplete {
            library: library.state(),
            queue_entries: self.entries.len(),
            played_threshold_percent: library.played_threshold_percent(),
        }
    }

    fn persist_state(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(state_path) = &self.state_path else {
            self.dirty = false;
            return Ok(());
        };
        let parent = state_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true).mode(0o700).create(parent)?;
        let bytes = serde_json::to_vec(&PersistedQueueState {
            version: QUEUE_STATE_VERSION,
            entries: self.entries.clone(),
            current_entry_id: self.current_entry_id,
            next_entry_id: self.next_entry_id,
            repeat_mode: self.repeat_mode,
            shuffle: self.shuffle,
        })
        .map_err(|error| io::Error::other(format!("could not encode queue state: {error}")))?;
        let temp_path = queue_state_temp_path(state_path);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp_path, state_path)?;
            File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        } else {
            self.dirty = false;
        }
        result
    }

    fn persisted_queue_state(&self) -> PersistedQueueState {
        PersistedQueueState {
            version: QUEUE_STATE_VERSION,
            entries: self.entries.clone(),
            current_entry_id: self.current_entry_id,
            next_entry_id: self.next_entry_id,
            repeat_mode: self.repeat_mode,
            shuffle: self.shuffle,
        }
    }

    fn export_app_state(&self, path: &std::path::Path, library: &LibraryService) -> io::Result<()> {
        let portable = PortableAppState {
            version: PORTABLE_STATE_VERSION,
            library: library.persisted_state(),
            queue: self.persisted_queue_state(),
        };
        write_portable_state(path, &portable)
    }

    fn import_app_state(
        &mut self,
        path: &std::path::Path,
        library: &mut LibraryService,
    ) -> io::Result<()> {
        let bytes = fs::read(path)?;
        let portable: PortableAppState = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("could not read exported Kulkán state: {error}"),
            )
        })?;
        if portable.version != PORTABLE_STATE_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported exported state version {}", portable.version),
            ));
        }
        validate_queue_state(&portable.queue)?;
        self.observe_current(library).map_err(io::Error::other)?;
        let old_library = library.persisted_state();
        let old_queue = self.persisted_queue_state();
        library.replace_persisted_state(portable.library)?;
        if let Err(error) = self.replace_queue_state(portable.queue, library) {
            let _ = library.replace_persisted_state(old_library);
            let _ = self.replace_queue_state(old_queue, library);
            return Err(io::Error::other(error));
        }
        if let Err(error) = self.persist_state() {
            let _ = library.replace_persisted_state(old_library);
            let _ = self.replace_queue_state(old_queue, library);
            return Err(error);
        }
        Ok(())
    }

    fn reset_app_state(&mut self, library: &mut LibraryService) -> Result<(), String> {
        let old_library = library.persisted_state();
        let old_queue = self.persisted_queue_state();
        library.reset().map_err(|error| error.to_string())?;
        let empty_queue = PersistedQueueState {
            version: QUEUE_STATE_VERSION,
            entries: Vec::new(),
            current_entry_id: None,
            next_entry_id: 1,
            repeat_mode: RepeatMode::Off,
            shuffle: false,
        };
        if let Err(error) = self.replace_queue_state(empty_queue, library) {
            let _ = library.replace_persisted_state(old_library);
            let _ = self.replace_queue_state(old_queue, library);
            return Err(error);
        }
        if let Err(error) = self.persist_state() {
            let _ = library.replace_persisted_state(old_library);
            let _ = self.replace_queue_state(old_queue, library);
            return Err(error.to_string());
        }
        Ok(())
    }

    fn replace_queue_state(
        &mut self,
        state: PersistedQueueState,
        library: &mut LibraryService,
    ) -> Result<(), String> {
        let _ = self.playback.handle(Message::StopPlayback);
        self.entries = state.entries;
        self.current_entry_id = state.current_entry_id;
        self.next_entry_id = state.next_entry_id.max(1);
        self.repeat_mode = state.repeat_mode;
        self.shuffle = state.shuffle;
        self.last_failure = None;
        self.attempt = None;
        self.pending_play_count_updates.clear();
        self.dirty = true;
        if let Some(entry_id) = self.current_entry_id
            && self
                .entries
                .iter()
                .any(|entry| entry.id == entry_id && !entry.failed)
        {
            self.play_entry(entry_id, library)?;
        }
        Ok(())
    }

    fn observe_current(
        &mut self,
        library: &mut LibraryService,
    ) -> Result<PlaybackSnapshot, String> {
        let snapshot = self.playback.state();
        self.observe_snapshot(library, &snapshot)?;
        Ok(snapshot)
    }

    fn observe_snapshot(
        &mut self,
        library: &mut LibraryService,
        snapshot: &PlaybackSnapshot,
    ) -> Result<(), String> {
        let newly_qualified = self.attempt.as_mut().and_then(|attempt| {
            attempt
                .observe(snapshot)
                .then(|| attempt.track().to_path_buf())
        });

        if let Some(track) = newly_qualified {
            if let Some(play_count) = library.record_qualifying_play(&track)? {
                self.pending_play_count_updates.push(PlayCountUpdate {
                    track: track.clone(),
                    play_count,
                });
            }
            if let Some(attempt) = self.attempt.as_mut() {
                attempt.mark_qualified();
            }
        }
        if snapshot.status == PlaybackStatus::Failed {
            self.attempt = None;
        }
        Ok(())
    }

    fn begin_attempt(&mut self, track: PathBuf, library: &LibraryService, position_ms: u64) {
        self.attempt = Some(PlaybackAttempt::new(
            track,
            library.played_threshold_percent(),
            position_ms,
        ));
    }

    fn play_track(&mut self, track: PathBuf, library: &mut LibraryService) -> Message {
        if let Err(error) = self.observe_current(library) {
            return Message::PlaybackOperationFailed(error);
        }
        match self.playback.handle(Message::PlayTrack(track.clone())) {
            Message::PlaybackState(snapshot) => {
                self.begin_attempt(track, library, snapshot.position_ms);
                Message::PlaybackState(snapshot)
            }
            response => {
                self.attempt = None;
                response
            }
        }
    }

    fn change_playback(&mut self, request: Message, library: &mut LibraryService) -> Message {
        if let Err(error) = self.observe_current(library) {
            return Message::PlaybackOperationFailed(error);
        }
        match self.playback.handle(request) {
            Message::PlaybackState(snapshot) => {
                if let Err(error) = self.observe_snapshot(library, &snapshot) {
                    return Message::PlaybackOperationFailed(error);
                }
                Message::PlaybackState(snapshot)
            }
            response => response,
        }
    }

    fn seek_playback(&mut self, position_ms: u64, library: &mut LibraryService) -> Message {
        if let Err(error) = self.observe_current(library) {
            return Message::PlaybackOperationFailed(error);
        }
        match self.playback.handle(Message::SeekPlayback(position_ms)) {
            Message::PlaybackState(snapshot) => {
                if let Some(attempt) = self.attempt.as_mut() {
                    attempt.seeked(position_ms, &snapshot);
                }
                Message::PlaybackState(snapshot)
            }
            response => response,
        }
    }

    fn stop_playback(&mut self, library: &mut LibraryService) -> Message {
        if let Err(error) = self.observe_current(library) {
            return Message::PlaybackOperationFailed(error);
        }
        self.attempt = None;
        self.playback.handle(Message::StopPlayback)
    }

    fn append(&mut self, tracks: Vec<PathBuf>) {
        for track in tracks {
            let id = self.next_entry_id;
            self.next_entry_id = self.next_entry_id.wrapping_add(1).max(1);
            self.entries.push(QueueEntry {
                id,
                track,
                failed: false,
            });
            self.dirty = true;
        }
    }

    fn replace_queue_and_play(
        &mut self,
        tracks: Vec<PathBuf>,
        library: &mut LibraryService,
    ) -> Result<(), String> {
        self.observe_current(library)?;
        self.playback.handle(Message::StopPlayback);
        self.entries.clear();
        self.current_entry_id = None;
        self.last_failure = None;
        self.attempt = None;
        self.dirty = true;
        self.append(tracks);
        if let Some(entry_id) = self.entries.first().map(|entry| entry.id) {
            self.play_entry(entry_id, library)?;
        }
        Ok(())
    }

    fn replace_first_and_play(
        &mut self,
        track: PathBuf,
        library: &mut LibraryService,
    ) -> Result<(), String> {
        self.observe_current(library)?;
        let index = self
            .current_entry_id
            .and_then(|current_id| self.entries.iter().position(|entry| entry.id == current_id))
            .unwrap_or(0);
        self.playback.handle(Message::StopPlayback);
        self.attempt = None;
        self.current_entry_id = None;

        let entry = QueueEntry {
            id: self.next_entry_id,
            track,
            failed: false,
        };
        self.next_entry_id = self.next_entry_id.wrapping_add(1).max(1);
        if self.entries.is_empty() {
            self.entries.push(entry.clone());
        } else {
            self.entries[index] = entry.clone();
        }
        self.dirty = true;
        self.play_entry(entry.id, library)
    }

    fn listen_next(&mut self, track: PathBuf, library: &mut LibraryService) -> Result<(), String> {
        self.observe_current(library)?;
        let was_playing = self.current_entry_id.is_some();
        let insert_at = self
            .current_entry_id
            .and_then(|current_id| self.entries.iter().position(|entry| entry.id == current_id))
            .map_or(0, |index| index + 1);
        let entry = QueueEntry {
            id: self.next_entry_id,
            track,
            failed: false,
        };
        self.next_entry_id = self.next_entry_id.wrapping_add(1).max(1);
        self.entries.insert(insert_at, entry.clone());
        self.dirty = true;
        if !was_playing {
            self.play_entry(entry.id, library)?;
        }
        Ok(())
    }

    fn play_entry(&mut self, entry_id: u64, library: &mut LibraryService) -> Result<(), String> {
        self.play_entry_with_mode(entry_id, library, false)
    }

    fn play_entry_with_mode(
        &mut self,
        entry_id: u64,
        library: &mut LibraryService,
        start_paused: bool,
    ) -> Result<(), String> {
        if !self.entries.iter().any(|entry| entry.id == entry_id) {
            return Err(format!("Queue entry {entry_id} does not exist"));
        }
        match self.start_entry(entry_id, library, start_paused) {
            Ok(()) => {
                self.dirty = true;
                Ok(())
            }
            Err(_) => {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == entry_id) {
                    entry.failed = true;
                    self.dirty = true;
                }
                self.advance(AdvanceKind::Failed, library, start_paused);
                Ok(())
            }
        }
    }

    fn reorder_entry(&mut self, entry_id: u64, position: usize) -> Result<(), String> {
        let from = self
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Queue entry {entry_id} does not exist"))?;
        let first_upcoming = self
            .current_entry_id
            .and_then(|id| self.entries.iter().position(|entry| entry.id == id))
            .map_or(0, |index| index + 1);
        if from < first_upcoming || position < first_upcoming {
            return Err("only upcoming queue entries can be reordered".to_owned());
        }
        if position >= self.entries.len() {
            return Err("queue position is outside the current Queue".to_owned());
        }
        let entry = self.entries.remove(from);
        self.entries.insert(position, entry);
        self.dirty = true;
        Ok(())
    }

    fn remove_entry(&mut self, entry_id: u64, library: &mut LibraryService) -> Result<(), String> {
        self.observe_current(library)?;
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Queue entry {entry_id} does not exist"))?;
        let was_current = self.current_entry_id == Some(entry_id);
        self.entries.remove(index);
        self.dirty = true;
        if was_current {
            let _ = self.playback.handle(Message::StopPlayback);
            self.attempt = None;
            self.current_entry_id = None;
            if !self.entries.is_empty() {
                let target_index = index.min(self.entries.len());
                if let Some(entry) = self.entries.get(target_index) {
                    let _ = self.play_entry(entry.id, library);
                } else if self.repeat_mode == RepeatMode::All {
                    self.advance(AdvanceKind::Skipped, library, false);
                }
            }
        }
        Ok(())
    }

    fn previous(&mut self, library: &mut LibraryService) -> Result<(), String> {
        let state = self.observe_current(library)?;
        self.advance_after_completion(state, library)?;
        let Some(current_id) = self.current_entry_id else {
            return Ok(());
        };
        let Some(current_index) = self.entries.iter().position(|entry| entry.id == current_id)
        else {
            return Ok(());
        };
        let target_id = if self.playback.state().position_ms < 2_000 {
            current_index
                .checked_sub(1)
                .and_then(|previous_index| self.entries.get(previous_index))
                .map_or(current_id, |entry| entry.id)
        } else {
            current_id
        };
        self.play_entry(target_id, library)
    }

    fn shuffle_range(&mut self, start: usize, end: usize) {
        let mut state = random_seed();
        for index in (start.saturating_add(1)..end).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let swap_index = start + (state as usize % (index - start + 1));
            self.entries.swap(index, swap_index);
        }
    }

    fn snapshot(&mut self, library: &mut LibraryService) -> Result<QueueSnapshot, String> {
        let state = self.observe_current(library)?;
        self.advance_after_completion(state, library)?;
        let playback = self.observe_current(library)?;
        Ok(QueueSnapshot {
            entries: self.entries.clone(),
            current_entry_id: self.current_entry_id,
            repeat_mode: self.repeat_mode,
            shuffle: self.shuffle,
            playback,
            play_count_updates: std::mem::take(&mut self.pending_play_count_updates),
        })
    }

    fn advance_after_completion(
        &mut self,
        state: PlaybackSnapshot,
        library: &mut LibraryService,
    ) -> Result<(), String> {
        match state.status {
            PlaybackStatus::Ended => self.advance(AdvanceKind::Finished, library, false),
            PlaybackStatus::Failed => {
                self.last_failure = state.error.clone();
                if let Some(current_id) = self.current_entry_id {
                    if let Some(entry) =
                        self.entries.iter_mut().find(|entry| entry.id == current_id)
                    {
                        entry.failed = true;
                        self.dirty = true;
                    }
                }
                self.advance(AdvanceKind::Failed, library, false);
            }
            _ => {}
        }
        Ok(())
    }

    fn advance(&mut self, kind: AdvanceKind, library: &mut LibraryService, start_paused: bool) {
        if kind == AdvanceKind::Failed && self.repeat_mode == RepeatMode::One {
            self.repeat_mode = RepeatMode::Off;
            self.dirty = true;
        }

        if kind == AdvanceKind::Finished && self.repeat_mode == RepeatMode::One {
            if let Some(current_id) = self.current_entry_id
                && self.start_entry(current_id, library, start_paused).is_ok()
            {
                return;
            }
            self.repeat_mode = RepeatMode::Off;
            if let Some(current_id) = self.current_entry_id {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == current_id) {
                    entry.failed = true;
                    self.dirty = true;
                }
            }
        }

        let current_index = self
            .current_entry_id
            .and_then(|id| self.entries.iter().position(|entry| entry.id == id));
        let start_index = current_index.map_or(0, |index| index.saturating_add(1));
        let mut candidates = (start_index..self.entries.len()).collect::<Vec<_>>();
        let mut started_new_cycle = false;
        loop {
            for index in candidates {
                if self.entries[index].failed {
                    continue;
                }
                let entry_id = self.entries[index].id;
                if self.start_entry(entry_id, library, start_paused).is_ok() {
                    return;
                }
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.failed = true;
                    self.dirty = true;
                }
            }

            if self.repeat_mode == RepeatMode::All && !started_new_cycle {
                if self.shuffle {
                    self.shuffle_range(0, self.entries.len());
                    self.dirty = true;
                }
                candidates = (0..self.entries.len()).collect();
                started_new_cycle = true;
                continue;
            }
            break;
        }

        if kind == AdvanceKind::Finished
            && self.repeat_mode == RepeatMode::Off
            && self.last_failure.is_none()
        {
            return;
        }
        if kind == AdvanceKind::Skipped {
            let _ = self.playback.handle(Message::StopPlayback);
            self.current_entry_id = None;
            self.attempt = None;
            self.dirty = true;
        } else {
            let error = self
                .last_failure
                .clone()
                .or_else(|| self.playback.state().error)
                .unwrap_or_else(|| "No playable queue entries remain".to_owned());
            self.playback.report_queue_failure(error);
            self.attempt = None;
            self.dirty = true;
        }
    }

    fn start_entry(
        &mut self,
        entry_id: u64,
        library: &mut LibraryService,
        start_paused: bool,
    ) -> Result<(), String> {
        self.observe_current(library)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Queue entry {entry_id} does not exist"))?;
        entry.failed = false;
        let track = entry.track.clone();
        self.current_entry_id = Some(entry_id);
        self.dirty = true;
        let response = if start_paused {
            self.playback
                .start_paused(track.clone())
                .map(Message::PlaybackState)
                .unwrap_or_else(Message::PlaybackOperationFailed)
        } else {
            self.playback.handle(Message::PlayTrack(track.clone()))
        };
        match response {
            Message::PlaybackState(snapshot) => {
                self.last_failure = None;
                self.begin_attempt(track, library, snapshot.position_ms);
                Ok(())
            }
            Message::PlaybackOperationFailed(error) => {
                self.last_failure = Some(error.clone());
                self.attempt = None;
                Err(error)
            }
            _ => {
                self.attempt = None;
                Err("server returned an unexpected playback response".to_owned())
            }
        }
    }
}

fn queue_state_temp_path(state_path: &std::path::Path) -> PathBuf {
    let sequence = NEXT_QUEUE_STATE_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let mut name = state_path
        .file_name()
        .map_or_else(|| "queue-state".into(), |name| name.to_os_string());
    name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
    state_path.with_file_name(name)
}

fn validate_queue_state(state: &PersistedQueueState) -> io::Result<()> {
    let mut ids = HashSet::with_capacity(state.entries.len());
    if state.entries.iter().any(|entry| !ids.insert(entry.id)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "queue state contains duplicate entry identifiers",
        ));
    }
    if state.next_entry_id == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "queue state contains an invalid next entry identifier",
        ));
    }
    if state
        .current_entry_id
        .is_some_and(|current| !ids.contains(&current))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "queue state refers to a missing current entry",
        ));
    }
    Ok(())
}

fn write_portable_state(path: &std::path::Path, state: &PortableAppState) -> io::Result<()> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| io::Error::other(format!("could not encode exported state: {error}")))?;
    // A save-file portal grants access to the chosen file, not necessarily its parent directory.
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()
}

fn random_seed() -> u64 {
    let mut bytes = [0_u8; 8];
    if File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .is_ok()
    {
        return u64::from_ne_bytes(bytes).max(1);
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| duration.as_nanos() as u64)
        .max(1)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AdvanceKind {
    Finished,
    Failed,
    Skipped,
}
