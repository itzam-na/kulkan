use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use kulkan_ipc::{Message, QueueEntry, QueueSnapshot, RepeatMode};

use crate::playback::PlaybackService;

pub(super) struct QueueService {
    playback: PlaybackService,
    entries: Vec<QueueEntry>,
    current_entry_id: Option<u64>,
    next_entry_id: u64,
    repeat_mode: RepeatMode,
    shuffle: bool,
    last_failure: Option<String>,
}

impl QueueService {
    pub(super) fn new(audio_sink: Option<String>) -> Self {
        Self {
            playback: PlaybackService::new(audio_sink),
            entries: Vec::new(),
            current_entry_id: None,
            next_entry_id: 1,
            repeat_mode: RepeatMode::Off,
            shuffle: false,
            last_failure: None,
        }
    }

    pub(super) fn handles(message: &Message) -> bool {
        PlaybackService::handles(message)
            || matches!(
                message,
                Message::AppendQueue(_)
                    | Message::GetQueueState
                    | Message::PlayQueueEntry(_)
                    | Message::SkipQueueEntry
                    | Message::ReorderQueueEntry { .. }
                    | Message::RemoveQueueEntry(_)
                    | Message::ClearQueue
                    | Message::SetRepeatMode(_)
                    | Message::SetShuffle(_)
            )
    }

    pub(super) fn handle(&mut self, message: Message) -> Message {
        match message {
            Message::AppendQueue(tracks) => {
                self.append(tracks);
                Message::QueueState(self.snapshot())
            }
            Message::GetQueueState => Message::QueueState(self.snapshot()),
            Message::PlayQueueEntry(entry_id) => match self.play_entry(entry_id) {
                Ok(()) => Message::QueueState(self.snapshot()),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::SkipQueueEntry => {
                self.advance(AdvanceKind::Skipped);
                Message::QueueState(self.snapshot())
            }
            Message::ReorderQueueEntry { entry_id, position } => {
                match self.reorder_entry(entry_id, position) {
                    Ok(()) => Message::QueueState(self.snapshot()),
                    Err(error) => Message::QueueOperationFailed(error),
                }
            }
            Message::RemoveQueueEntry(entry_id) => match self.remove_entry(entry_id) {
                Ok(()) => Message::QueueState(self.snapshot()),
                Err(error) => Message::QueueOperationFailed(error),
            },
            Message::ClearQueue => {
                self.entries.clear();
                self.current_entry_id = None;
                self.last_failure = None;
                let _ = self.playback.handle(Message::StopPlayback);
                Message::QueueState(self.snapshot())
            }
            Message::SetRepeatMode(repeat_mode) => {
                self.repeat_mode = repeat_mode;
                Message::QueueState(self.snapshot())
            }
            Message::SetShuffle(enabled) => {
                if enabled && !self.shuffle {
                    let first_upcoming = self
                        .current_entry_id
                        .and_then(|id| self.entries.iter().position(|entry| entry.id == id))
                        .map_or(0, |index| index + 1);
                    self.shuffle_range(first_upcoming, self.entries.len());
                }
                self.shuffle = enabled;
                Message::QueueState(self.snapshot())
            }
            request if PlaybackService::handles(&request) => self.playback.handle(request),
            _ => Message::QueueOperationFailed("unsupported queue operation".to_owned()),
        }
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
        }
    }

    fn play_entry(&mut self, entry_id: u64) -> Result<(), String> {
        if !self.entries.iter().any(|entry| entry.id == entry_id) {
            return Err(format!("Queue entry {entry_id} does not exist"));
        }
        match self.start_entry(entry_id) {
            Ok(()) => Ok(()),
            Err(_) => {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == entry_id) {
                    entry.failed = true;
                }
                self.advance(AdvanceKind::Failed);
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
        Ok(())
    }

    fn remove_entry(&mut self, entry_id: u64) -> Result<(), String> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Queue entry {entry_id} does not exist"))?;
        let was_current = self.current_entry_id == Some(entry_id);
        self.entries.remove(index);
        if was_current {
            let _ = self.playback.handle(Message::StopPlayback);
            self.current_entry_id = None;
            if !self.entries.is_empty() {
                let target_index = index.min(self.entries.len());
                if let Some(entry) = self.entries.get(target_index) {
                    let _ = self.play_entry(entry.id);
                } else if self.repeat_mode == RepeatMode::All {
                    self.advance(AdvanceKind::Skipped);
                }
            }
        }
        Ok(())
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

    fn snapshot(&mut self) -> QueueSnapshot {
        self.advance_after_completion();
        QueueSnapshot {
            entries: self.entries.clone(),
            current_entry_id: self.current_entry_id,
            repeat_mode: self.repeat_mode,
            shuffle: self.shuffle,
            playback: self.playback.state(),
        }
    }

    fn advance_after_completion(&mut self) {
        let state = self.playback.state();
        match state.status {
            kulkan_ipc::PlaybackStatus::Ended => self.advance(AdvanceKind::Finished),
            kulkan_ipc::PlaybackStatus::Failed => {
                self.last_failure = state.error.clone();
                if let Some(current_id) = self.current_entry_id {
                    if let Some(entry) =
                        self.entries.iter_mut().find(|entry| entry.id == current_id)
                    {
                        entry.failed = true;
                    }
                }
                self.advance(AdvanceKind::Failed);
            }
            _ => {}
        }
    }

    fn advance(&mut self, kind: AdvanceKind) {
        if kind == AdvanceKind::Failed && self.repeat_mode == RepeatMode::One {
            self.repeat_mode = RepeatMode::Off;
        }

        if kind == AdvanceKind::Finished && self.repeat_mode == RepeatMode::One {
            if let Some(current_id) = self.current_entry_id
                && self.start_entry(current_id).is_ok()
            {
                return;
            }
            self.repeat_mode = RepeatMode::Off;
            if let Some(current_id) = self.current_entry_id {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == current_id) {
                    entry.failed = true;
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
                if self.start_entry(entry_id).is_ok() {
                    return;
                }
                if let Some(entry) = self.entries.get_mut(index) {
                    entry.failed = true;
                }
            }

            if self.repeat_mode == RepeatMode::All && !started_new_cycle {
                if self.shuffle {
                    self.shuffle_range(0, self.entries.len());
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
        } else {
            let error = self
                .last_failure
                .clone()
                .or_else(|| self.playback.state().error)
                .unwrap_or_else(|| "No playable queue entries remain".to_owned());
            self.playback.report_queue_failure(error);
        }
    }

    fn start_entry(&mut self, entry_id: u64) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("Queue entry {entry_id} does not exist"))?;
        entry.failed = false;
        let track = entry.track.clone();
        self.current_entry_id = Some(entry_id);
        match self.playback.handle(Message::PlayTrack(track)) {
            Message::PlaybackState(_) => {
                self.last_failure = None;
                Ok(())
            }
            Message::PlaybackOperationFailed(error) => {
                self.last_failure = Some(error.clone());
                Err(error)
            }
            _ => Err("server returned an unexpected playback response".to_owned()),
        }
    }
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
