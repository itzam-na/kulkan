use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kulkan_ipc::{
    Message, ProtocolError, bind_private_socket, peer_pid, read_message, validate_peer,
    write_message,
};
pub use kulkan_ipc::{
    PlayCountUpdate, PlaybackSnapshot, PlaybackStatus, QueueEntry, QueueSnapshot, RepeatMode,
};
pub use kulkan_library::SearchCriteria;
use kulkan_library::{
    Catalog, CatalogPage, LibraryState, ScanSummary, scan_library, search_catalog,
};
use serde::{Deserialize, Serialize};

mod playback;
mod queue;

const MAX_CATALOG_PAGE_SIZE: usize = 32;
const LIBRARY_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const LIBRARY_STATE_VERSION: u16 = 2;
const DEFAULT_PLAYED_THRESHOLD_PERCENT: u8 = 75;
static NEXT_LIBRARY_STATE_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Runs the local server protocol until the client asks it to shut down.
pub fn serve(socket_path: &Path) -> io::Result<()> {
    serve_with_audio_sink(socket_path, None)
}

/// Runs the local server with an optional named GStreamer audio sink.
///
/// The sink override is intended for headless integration tests; production uses GStreamer's
/// automatically selected host audio sink.
pub fn serve_with_audio_sink(socket_path: &Path, audio_sink: Option<&str>) -> io::Result<()> {
    serve_with_library_state(socket_path, audio_sink, None)
}

/// Runs the local server and restores/saves selected roots and the scanned catalog at `state_path`.
pub fn serve_with_library_state(
    socket_path: &Path,
    audio_sink: Option<&str>,
    state_path: Option<&Path>,
) -> io::Result<()> {
    let listener = bind_private_socket(socket_path)?;
    let _cleanup = SocketPathCleanup(socket_path.to_path_buf());
    let mut library = LibraryService::load(state_path)?;
    let queue_state_path = state_path.map(|path| path.with_file_name("queue.json"));
    let mut queue = queue::QueueService::load(
        audio_sink.map(str::to_owned),
        queue_state_path,
        &mut library,
    )?;

    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if validate_peer(&stream).is_err() {
            continue;
        }

        let request = match read_message(&mut stream) {
            Ok(request) => request,
            Err(_) => continue,
        };
        match request {
            Message::Ping => {
                if let Err(error) = write_server_response(&mut stream, &Message::Pong) {
                    if client_disconnected(&error) {
                        continue;
                    }
                    return Err(error);
                }
            }
            Message::Shutdown => {
                write_server_response(&mut stream, &Message::ShutdownAck)?;
                return Ok(());
            }
            request => {
                let response = if queue::QueueService::handles(&request) {
                    queue.handle(request, &mut library)
                } else {
                    library.handle(request)
                };
                if let Err(error) = write_server_response(&mut stream, &response) {
                    if client_disconnected(&error) {
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

/// Owns the child server process and its private socket connection lifecycle.
pub struct ServerProcess {
    child: Child,
    socket_path: PathBuf,
}

/// A cloneable IPC client for catalog operations on the managed child server.
#[derive(Clone, Debug)]
pub struct ServerClient {
    socket_path: PathBuf,
    server_pid: u32,
}

struct LibraryService {
    selected_roots: Vec<PathBuf>,
    catalog: Catalog,
    played_threshold_percent: u8,
    state_path: Option<PathBuf>,
}

#[derive(Clone, Deserialize, Serialize)]
struct PersistedLibraryState {
    version: u16,
    selected_roots: Vec<PathBuf>,
    catalog: Catalog,
    #[serde(default = "default_played_threshold_percent")]
    played_threshold_percent: u8,
}

fn default_played_threshold_percent() -> u8 {
    DEFAULT_PLAYED_THRESHOLD_PERCENT
}

impl LibraryService {
    fn load(state_path: Option<&Path>) -> io::Result<Self> {
        let Some(state_path) = state_path else {
            return Ok(Self {
                selected_roots: Vec::new(),
                catalog: Catalog::default(),
                played_threshold_percent: DEFAULT_PLAYED_THRESHOLD_PERCENT,
                state_path: None,
            });
        };
        match fs::read(state_path) {
            Ok(bytes) => {
                let state: PersistedLibraryState =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "could not read library state {}: {error}",
                                state_path.display()
                            ),
                        )
                    })?;
                if !matches!(state.version, 1 | LIBRARY_STATE_VERSION) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported library state version {} in {}",
                            state.version,
                            state_path.display()
                        ),
                    ));
                }
                Ok(Self {
                    selected_roots: state.selected_roots,
                    catalog: state.catalog,
                    played_threshold_percent: state.played_threshold_percent,
                    state_path: Some(state_path.to_path_buf()),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self {
                selected_roots: Vec::new(),
                catalog: Catalog::default(),
                played_threshold_percent: DEFAULT_PLAYED_THRESHOLD_PERCENT,
                state_path: Some(state_path.to_path_buf()),
            }),
            Err(error) => Err(error),
        }
    }

    fn persist(&self) -> io::Result<()> {
        let Some(state_path) = &self.state_path else {
            return Ok(());
        };
        let parent = state_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true).mode(0o700).create(parent)?;
        let bytes = serde_json::to_vec(&PersistedLibraryState {
            version: LIBRARY_STATE_VERSION,
            selected_roots: self.selected_roots.clone(),
            catalog: self.catalog.clone(),
            played_threshold_percent: self.played_threshold_percent,
        })
        .map_err(|error| io::Error::other(format!("could not encode library state: {error}")))?;
        let temp_path = library_state_temp_path(state_path);
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
        }
        result
    }

    fn persisted_state(&self) -> PersistedLibraryState {
        PersistedLibraryState {
            version: LIBRARY_STATE_VERSION,
            selected_roots: self.selected_roots.clone(),
            catalog: self.catalog.clone(),
            played_threshold_percent: self.played_threshold_percent,
        }
    }

    fn replace_persisted_state(&mut self, mut state: PersistedLibraryState) -> io::Result<()> {
        if !matches!(state.version, 1 | LIBRARY_STATE_VERSION) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported library state version {}", state.version),
            ));
        }
        if !(1..=100).contains(&state.played_threshold_percent) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "played threshold in imported state must be from 1 through 100",
            ));
        }
        let old_state = self.persisted_state();
        state.version = LIBRARY_STATE_VERSION;
        self.selected_roots = state.selected_roots;
        self.catalog = state.catalog;
        self.played_threshold_percent = state.played_threshold_percent;
        if let Err(error) = self.persist() {
            self.selected_roots = old_state.selected_roots;
            self.catalog = old_state.catalog;
            self.played_threshold_percent = old_state.played_threshold_percent;
            return Err(error);
        }
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        let old_state = self.persisted_state();
        self.selected_roots.clear();
        self.catalog = Catalog::default();
        self.played_threshold_percent = DEFAULT_PLAYED_THRESHOLD_PERCENT;
        if let Err(error) = self.persist() {
            self.selected_roots = old_state.selected_roots;
            self.catalog = old_state.catalog;
            self.played_threshold_percent = old_state.played_threshold_percent;
            return Err(error);
        }
        Ok(())
    }
}

impl LibraryService {
    fn played_threshold_percent(&self) -> u8 {
        self.played_threshold_percent
    }

    fn handle(&mut self, request: Message) -> Message {
        match request {
            Message::AddLibraryRoot(path) => {
                let path = absolute_path(&path);
                if !self.selected_roots.contains(&path) {
                    self.selected_roots.push(path);
                    if let Err(error) = self.persist() {
                        self.selected_roots.pop();
                        return Message::LibraryOperationFailed(error.to_string());
                    }
                }
                self.state_message()
            }
            Message::RemoveLibraryRoot(path) => {
                let path = absolute_path(&path);
                let previous_roots = self.selected_roots.clone();
                self.selected_roots.retain(|root| root != &path);
                if self.selected_roots != previous_roots
                    && let Err(error) = self.persist()
                {
                    self.selected_roots = previous_roots;
                    return Message::LibraryOperationFailed(error.to_string());
                }
                self.state_message()
            }
            Message::GetLibraryState => self.state_message(),
            Message::GetPlayedThreshold => Message::PlayedThreshold(self.played_threshold_percent),
            Message::SetPlayedThreshold(percent) => {
                if !(1..=100).contains(&percent) {
                    return Message::LibraryOperationFailed(
                        "played threshold must be an integer from 1 through 100".to_owned(),
                    );
                }
                let previous = self.played_threshold_percent;
                self.played_threshold_percent = percent;
                if let Err(error) = self.persist() {
                    self.played_threshold_percent = previous;
                    return Message::LibraryOperationFailed(error.to_string());
                }
                Message::PlayedThreshold(percent)
            }
            Message::ClearEntryPlayHistory(path) => match self.clear_play_history(Some(&path)) {
                Ok(()) => self.state_message(),
                Err(error) => Message::LibraryOperationFailed(error),
            },
            Message::ClearAllPlayHistory => match self.clear_play_history(None) {
                Ok(()) => self.state_message(),
                Err(error) => Message::LibraryOperationFailed(error),
            },
            Message::GetCatalogPage { offset, limit } => {
                if limit == 0 || limit > MAX_CATALOG_PAGE_SIZE {
                    return Message::LibraryOperationFailed(format!(
                        "catalog page size must be between 1 and {MAX_CATALOG_PAGE_SIZE}"
                    ));
                }
                let start = offset.min(self.catalog.entries.len());
                let end = start.saturating_add(limit).min(self.catalog.entries.len());
                Message::CatalogPage(CatalogPage {
                    entries: self.catalog.entries[start..end].to_vec(),
                    next_offset: (end < self.catalog.entries.len()).then_some(end),
                })
            }
            Message::SearchCatalog {
                criteria,
                offset,
                limit,
            } => {
                if limit == 0 || limit > MAX_CATALOG_PAGE_SIZE {
                    return Message::LibraryOperationFailed(format!(
                        "catalog page size must be between 1 and {MAX_CATALOG_PAGE_SIZE}"
                    ));
                }
                let results = match search_catalog(&self.catalog, &criteria) {
                    Ok(results) => results,
                    Err(error) => return Message::LibraryOperationFailed(error),
                };
                let start = offset.min(results.len());
                let end = start.saturating_add(limit).min(results.len());
                Message::CatalogPage(CatalogPage {
                    entries: results[start..end].to_vec(),
                    next_offset: (end < results.len()).then_some(end),
                })
            }
            Message::ScanLibrary => {
                let result = scan_library(&self.catalog, &self.selected_roots);
                let previous_catalog = std::mem::replace(&mut self.catalog, result.catalog);
                if let Err(error) = self.persist() {
                    self.catalog = previous_catalog;
                    return Message::LibraryOperationFailed(error.to_string());
                }
                let active_entries = self
                    .catalog
                    .entries
                    .iter()
                    .filter(|entry| entry.active)
                    .count();
                let inactive_entries = self.catalog.entries.len() - active_entries;
                Message::ScanSummary(ScanSummary {
                    incomplete_paths: result.incomplete_paths,
                    media_errors: result.media_errors,
                    active_entries,
                    inactive_entries,
                })
            }
            _ => Message::LibraryOperationFailed("unsupported library operation".to_owned()),
        }
    }

    fn state_message(&self) -> Message {
        Message::LibraryState(self.state())
    }

    fn state(&self) -> LibraryState {
        LibraryState {
            selected_roots: self.selected_roots.clone(),
            catalog_entries: self.catalog.entries.len(),
        }
    }

    fn record_qualifying_play(&mut self, path: &Path) -> Result<Option<u64>, String> {
        let Some(index) = self
            .catalog
            .entries
            .iter()
            .position(|entry| entry.path == path)
        else {
            return Ok(None);
        };
        let previous_count = self.catalog.entries[index].play_count;
        self.catalog.entries[index].play_count = previous_count.saturating_add(1);
        if self.catalog.entries[index].play_count == previous_count {
            return Ok(None);
        }
        if let Err(error) = self.persist() {
            self.catalog.entries[index].play_count = previous_count;
            return Err(error.to_string());
        }
        Ok(Some(self.catalog.entries[index].play_count))
    }

    fn clear_play_history(&mut self, path: Option<&Path>) -> Result<(), String> {
        let path = path.map(absolute_path);
        let old_catalog = self.catalog.clone();
        if let Some(path) = &path {
            let Some(entry) = self
                .catalog
                .entries
                .iter_mut()
                .find(|entry| &entry.path == path)
            else {
                return Err(format!("Library entry does not exist: {}", path.display()));
            };
            entry.play_count = 0;
        } else {
            for entry in &mut self.catalog.entries {
                entry.play_count = 0;
            }
        }
        if let Err(error) = self.persist() {
            self.catalog = old_catalog;
            return Err(error.to_string());
        }
        Ok(())
    }
}

fn library_state_temp_path(state_path: &Path) -> PathBuf {
    let sequence = NEXT_LIBRARY_STATE_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let mut name = state_path
        .file_name()
        .map_or_else(|| "library-state".into(), |name| name.to_os_string());
    name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
    state_path.with_file_name(name)
}

/// Owns the private activation socket for the one running desktop client.
pub struct ClientInstance {
    socket_path: PathBuf,
    shutting_down: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
}

impl ClientInstance {
    /// Claims the client activation socket and returns notifications for new launches.
    pub fn start(socket_path: &Path) -> io::Result<(Self, mpsc::Receiver<()>)> {
        let listener = bind_private_socket(socket_path)?;
        listener.set_nonblocking(true)?;
        let shutting_down = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutting_down);
        let (activation_sender, activation_receiver) = mpsc::channel();
        let listener_thread = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if validate_peer(&stream).is_err() {
                            continue;
                        }
                        if stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .is_err()
                        {
                            continue;
                        }
                        if read_message(&mut stream)
                            .is_ok_and(|message| message == Message::Activate)
                            && write_message(&mut stream, &Message::ActivateAck).is_ok()
                        {
                            let _ = activation_sender.send(());
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok((
            Self {
                socket_path: socket_path.to_path_buf(),
                shutting_down,
                listener_thread: Some(listener_thread),
            },
            activation_receiver,
        ))
    }
}

impl Drop for ClientInstance {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::Relaxed);
        if let Some(listener_thread) = self.listener_thread.take() {
            let _ = listener_thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Asks an already-running client to present its window.
///
/// Returns `Ok(false)` if no live client currently owns the socket.
pub fn activate_existing_client(socket_path: &Path) -> io::Result<bool> {
    let mut client = match UnixStream::connect(socket_path) {
        Ok(client) => client,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    validate_peer(&client)?;
    write_message(&mut client, &Message::Activate)
        .map_err(|error| io::Error::other(error.to_string()))?;
    match read_message(&mut client).map_err(|error| io::Error::other(error.to_string()))? {
        Message::ActivateAck => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "existing client returned an unexpected activation response",
        )),
    }
}

impl ServerProcess {
    /// Starts the server executable and waits until its versioned socket responds to Ping.
    pub fn start(
        executable: &Path,
        socket_path: &Path,
        startup_timeout: Duration,
    ) -> io::Result<Self> {
        Self::start_with_audio_sink(executable, socket_path, startup_timeout, None)
    }

    /// Starts the server with an explicit GStreamer audio sink, primarily for headless tests.
    pub fn start_with_audio_sink(
        executable: &Path,
        socket_path: &Path,
        startup_timeout: Duration,
        audio_sink: Option<&str>,
    ) -> io::Result<Self> {
        Self::start_with_options(executable, socket_path, startup_timeout, audio_sink, None)
    }

    /// Starts the server with persistent selected roots and scanned library state.
    pub fn start_with_library_state(
        executable: &Path,
        socket_path: &Path,
        state_path: &Path,
        startup_timeout: Duration,
    ) -> io::Result<Self> {
        Self::start_with_options(
            executable,
            socket_path,
            startup_timeout,
            None,
            Some(state_path),
        )
    }

    /// Starts the server with persistent library state and an explicit GStreamer audio sink.
    pub fn start_with_audio_sink_and_library_state(
        executable: &Path,
        socket_path: &Path,
        state_path: &Path,
        startup_timeout: Duration,
        audio_sink: &str,
    ) -> io::Result<Self> {
        Self::start_with_options(
            executable,
            socket_path,
            startup_timeout,
            Some(audio_sink),
            Some(state_path),
        )
    }

    fn start_with_options(
        executable: &Path,
        socket_path: &Path,
        startup_timeout: Duration,
        audio_sink: Option<&str>,
        state_path: Option<&Path>,
    ) -> io::Result<Self> {
        stop_existing_server(socket_path, startup_timeout)?;
        let mut command = Command::new(executable);
        command.arg("--socket").arg(socket_path);
        if let Some(audio_sink) = audio_sink {
            command.arg("--audio-sink").arg(audio_sink);
        }
        if let Some(state_path) = state_path {
            command.arg("--library-state").arg(state_path);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + startup_timeout;

        loop {
            if let Ok(mut server) = connect(socket_path) {
                if peer_pid(&server)? != child.id() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "a different process owns the server socket",
                    ));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                set_io_timeout(&server, remaining.min(Duration::from_millis(100)))?;
                if write_message(&mut server, &Message::Ping).is_ok()
                    && read_message(&mut server).is_ok_and(|message| message == Message::Pong)
                {
                    return Ok(Self {
                        child,
                        socket_path: socket_path.to_path_buf(),
                    });
                }
            }

            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!(
                    "server exited during startup with {status}"
                )));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "server did not become ready before the startup timeout",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Confirms the child still serves the current protocol.
    pub fn ping(&self) -> io::Result<()> {
        let mut server = self.connect_to_child()?;
        set_io_timeout(&server, Duration::from_secs(2))?;
        write_message(&mut server, &Message::Ping)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match read_message(&mut server).map_err(|error| io::Error::other(error.to_string()))? {
            Message::Pong => Ok(()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "server returned an unexpected response to Ping",
            )),
        }
    }

    /// Requests a clean shutdown and waits for the child to exit.
    pub fn shutdown(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }

        let mut server = self.connect_to_child()?;
        set_io_timeout(&server, Duration::from_secs(2))?;
        write_message(&mut server, &Message::Shutdown)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match read_message(&mut server).map_err(|error| io::Error::other(error.to_string()))? {
            Message::ShutdownAck => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "server returned an unexpected response to Shutdown",
                ));
            }
        }
        self.wait_until_exit(Duration::from_secs(2))
    }

    /// Reports whether the child process is still running.
    pub fn is_running(&mut self) -> io::Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    /// Returns an IPC client tied to this child server's process identity.
    pub fn client(&self) -> ServerClient {
        ServerClient {
            socket_path: self.socket_path.clone(),
            server_pid: self.child.id(),
        }
    }

    /// Adds a selected root without scanning it.
    pub fn add_library_root(&self, path: &Path) -> io::Result<LibraryState> {
        self.client().add_library_root(path)
    }

    /// Removes a selected root; entries are reconciled on the next explicit scan.
    pub fn remove_library_root(&self, path: &Path) -> io::Result<LibraryState> {
        self.client().remove_library_root(path)
    }

    /// Runs one explicit scan of the selected roots.
    pub fn scan_library(&self) -> io::Result<ScanSummary> {
        self.client().scan_library()
    }

    /// Returns the current root selection and the number of catalog entries.
    pub fn library_state(&self) -> io::Result<LibraryState> {
        self.client().library_state()
    }

    pub fn played_threshold(&self) -> io::Result<u8> {
        self.client().played_threshold()
    }

    pub fn set_played_threshold(&self, percent: u8) -> io::Result<u8> {
        self.client().set_played_threshold(percent)
    }

    pub fn clear_entry_play_history(&self, path: &Path) -> io::Result<LibraryState> {
        self.client().clear_entry_play_history(path)
    }

    pub fn clear_all_play_history(&self) -> io::Result<LibraryState> {
        self.client().clear_all_play_history()
    }

    pub fn export_app_state(&self, path: &Path) -> io::Result<()> {
        self.client().export_app_state(path)
    }

    pub fn import_app_state(&self, path: &Path) -> io::Result<LibraryState> {
        self.client().import_app_state(path)
    }

    pub fn reset_app_state(&self) -> io::Result<LibraryState> {
        self.client().reset_app_state()
    }

    /// Reads one bounded page of catalog entries, including inactive records.
    pub fn catalog_page(&self, offset: usize, limit: usize) -> io::Result<CatalogPage> {
        self.client().catalog_page(offset, limit)
    }

    pub fn search_catalog(
        &self,
        criteria: &SearchCriteria,
        offset: usize,
        limit: usize,
    ) -> io::Result<CatalogPage> {
        self.client().search_catalog(criteria, offset, limit)
    }

    pub fn play_track(&self, path: &Path) -> io::Result<PlaybackSnapshot> {
        self.client().play_track(path)
    }

    pub fn pause_playback(&self) -> io::Result<PlaybackSnapshot> {
        self.client().pause_playback()
    }

    pub fn resume_playback(&self) -> io::Result<PlaybackSnapshot> {
        self.client().resume_playback()
    }

    pub fn seek_playback(&self, position_ms: u64) -> io::Result<PlaybackSnapshot> {
        self.client().seek_playback(position_ms)
    }

    pub fn stop_playback(&self) -> io::Result<PlaybackSnapshot> {
        self.client().stop_playback()
    }

    pub fn playback_state(&self) -> io::Result<PlaybackSnapshot> {
        self.client().playback_state()
    }

    pub fn append_to_queue(&self, tracks: &[PathBuf]) -> io::Result<QueueSnapshot> {
        self.client().append_to_queue(tracks)
    }

    pub fn queue_state(&self) -> io::Result<QueueSnapshot> {
        self.client().queue_state()
    }

    pub fn play_queue_entry(&self, entry_id: u64) -> io::Result<QueueSnapshot> {
        self.client().play_queue_entry(entry_id)
    }

    pub fn previous_queue_entry(&self) -> io::Result<QueueSnapshot> {
        self.client().previous_queue_entry()
    }

    pub fn skip_queue_entry(&self) -> io::Result<QueueSnapshot> {
        self.client().skip_queue_entry()
    }

    pub fn reorder_queue_entry(&self, entry_id: u64, position: usize) -> io::Result<QueueSnapshot> {
        self.client().reorder_queue_entry(entry_id, position)
    }

    pub fn remove_queue_entry(&self, entry_id: u64) -> io::Result<QueueSnapshot> {
        self.client().remove_queue_entry(entry_id)
    }

    pub fn clear_queue(&self) -> io::Result<QueueSnapshot> {
        self.client().clear_queue()
    }

    pub fn set_repeat_mode(&self, repeat_mode: RepeatMode) -> io::Result<QueueSnapshot> {
        self.client().set_repeat_mode(repeat_mode)
    }

    pub fn set_shuffle(&self, enabled: bool) -> io::Result<QueueSnapshot> {
        self.client().set_shuffle(enabled)
    }

    fn connect_to_child(&self) -> io::Result<UnixStream> {
        let stream = connect(&self.socket_path)?;
        if peer_pid(&stream)? != self.child.id() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "server socket is no longer owned by the managed child",
            ));
        }
        Ok(stream)
    }

    fn wait_until_exit(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.child.kill()?;
                let _ = self.child.wait()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "server did not exit after acknowledging Shutdown",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl ServerClient {
    pub fn add_library_root(&self, path: &Path) -> io::Result<LibraryState> {
        expect_library_state(self.request(Message::AddLibraryRoot(path.to_path_buf()))?)
    }

    pub fn remove_library_root(&self, path: &Path) -> io::Result<LibraryState> {
        expect_library_state(self.request(Message::RemoveLibraryRoot(path.to_path_buf()))?)
    }

    pub fn scan_library(&self) -> io::Result<ScanSummary> {
        match self.request(Message::ScanLibrary)? {
            Message::ScanSummary(summary) => Ok(summary),
            response => unexpected_library_response(response),
        }
    }

    pub fn library_state(&self) -> io::Result<LibraryState> {
        expect_library_state(self.request(Message::GetLibraryState)?)
    }

    pub fn played_threshold(&self) -> io::Result<u8> {
        match self.request(Message::GetPlayedThreshold)? {
            Message::PlayedThreshold(percent) => Ok(percent),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn set_played_threshold(&self, percent: u8) -> io::Result<u8> {
        match self.request(Message::SetPlayedThreshold(percent))? {
            Message::PlayedThreshold(percent) => Ok(percent),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn clear_entry_play_history(&self, path: &Path) -> io::Result<LibraryState> {
        expect_library_state(self.request(Message::ClearEntryPlayHistory(path.to_path_buf()))?)
    }

    pub fn clear_all_play_history(&self) -> io::Result<LibraryState> {
        expect_library_state(self.request(Message::ClearAllPlayHistory)?)
    }

    pub fn export_app_state(&self, path: &Path) -> io::Result<()> {
        match self.request(Message::ExportAppState(path.to_path_buf()))? {
            Message::AppStateTransferComplete { .. } => Ok(()),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn import_app_state(&self, path: &Path) -> io::Result<LibraryState> {
        match self.request(Message::ImportAppState(path.to_path_buf()))? {
            Message::AppStateTransferComplete { library, .. } => Ok(library),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn reset_app_state(&self) -> io::Result<LibraryState> {
        match self.request(Message::ResetAppState)? {
            Message::AppStateTransferComplete { library, .. } => Ok(library),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn catalog_page(&self, offset: usize, limit: usize) -> io::Result<CatalogPage> {
        match self.request(Message::GetCatalogPage { offset, limit })? {
            Message::CatalogPage(page) => Ok(page),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn search_catalog(
        &self,
        criteria: &SearchCriteria,
        offset: usize,
        limit: usize,
    ) -> io::Result<CatalogPage> {
        match self.request(Message::SearchCatalog {
            criteria: criteria.clone(),
            offset,
            limit,
        })? {
            Message::CatalogPage(page) => Ok(page),
            Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
            response => unexpected_library_response(response),
        }
    }

    pub fn play_track(&self, path: &Path) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::PlayTrack(path.to_path_buf()))?)
    }

    pub fn pause_playback(&self) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::PausePlayback)?)
    }

    pub fn resume_playback(&self) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::ResumePlayback)?)
    }

    pub fn seek_playback(&self, position_ms: u64) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::SeekPlayback(position_ms))?)
    }

    pub fn stop_playback(&self) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::StopPlayback)?)
    }

    pub fn playback_state(&self) -> io::Result<PlaybackSnapshot> {
        expect_playback_state(self.request(Message::GetPlaybackState)?)
    }

    pub fn append_to_queue(&self, tracks: &[PathBuf]) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::AppendQueue(tracks.to_vec()))?)
    }

    pub fn queue_state(&self) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::GetQueueState)?)
    }

    pub fn play_queue_entry(&self, entry_id: u64) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::PlayQueueEntry(entry_id))?)
    }

    pub fn previous_queue_entry(&self) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::PreviousQueueEntry)?)
    }

    pub fn skip_queue_entry(&self) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::SkipQueueEntry)?)
    }

    pub fn reorder_queue_entry(&self, entry_id: u64, position: usize) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::ReorderQueueEntry { entry_id, position })?)
    }

    pub fn remove_queue_entry(&self, entry_id: u64) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::RemoveQueueEntry(entry_id))?)
    }

    pub fn clear_queue(&self) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::ClearQueue)?)
    }

    pub fn set_repeat_mode(&self, repeat_mode: RepeatMode) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::SetRepeatMode(repeat_mode))?)
    }

    pub fn set_shuffle(&self, enabled: bool) -> io::Result<QueueSnapshot> {
        expect_queue_state(self.request(Message::SetShuffle(enabled))?)
    }

    fn request(&self, request: Message) -> io::Result<Message> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        validate_peer(&stream)?;
        if peer_pid(&stream)? != self.server_pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "server socket is no longer owned by the managed child",
            ));
        }
        set_io_timeout(&stream, LIBRARY_REQUEST_TIMEOUT)?;
        write_message(&mut stream, &request)
            .map_err(|error| io::Error::other(error.to_string()))?;
        read_message(&mut stream).map_err(|error| io::Error::other(error.to_string()))
    }
}

fn expect_playback_state(response: Message) -> io::Result<PlaybackSnapshot> {
    match response {
        Message::PlaybackState(state) => Ok(state),
        Message::PlaybackOperationFailed(message) => Err(io::Error::other(message)),
        response => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("server returned an unexpected playback response: {response:?}"),
        )),
    }
}

fn expect_queue_state(response: Message) -> io::Result<QueueSnapshot> {
    match response {
        Message::QueueState(state) => Ok(state),
        Message::QueueOperationFailed(message) => Err(io::Error::other(message)),
        response => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("server returned an unexpected queue response: {response:?}"),
        )),
    }
}

fn expect_library_state(response: Message) -> io::Result<LibraryState> {
    match response {
        Message::LibraryState(state) => Ok(state),
        response => unexpected_library_response(response),
    }
}

fn unexpected_library_response<T>(response: Message) -> io::Result<T> {
    match response {
        Message::LibraryOperationFailed(message) => Err(io::Error::other(message)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("server returned an unexpected library response: {other:?}"),
        )),
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.shutdown();
        }
    }
}

fn connect(socket_path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket_path)?;
    validate_peer(&stream)?;
    Ok(stream)
}

fn stop_existing_server(socket_path: &Path, timeout: Duration) -> io::Result<()> {
    let mut server = match connect(socket_path) {
        Ok(server) => server,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    set_io_timeout(&server, timeout)?;
    write_message(&mut server, &Message::Shutdown)
        .map_err(|error| io::Error::other(error.to_string()))?;
    match read_message(&mut server).map_err(|error| io::Error::other(error.to_string()))? {
        Message::ShutdownAck => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "existing server returned an unexpected response to Shutdown",
            ));
        }
    }
    drop(server);

    let deadline = Instant::now() + timeout;
    while socket_path.exists() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "existing server did not release its socket",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn set_io_timeout(stream: &UnixStream, timeout: Duration) -> io::Result<()> {
    let timeout = timeout.max(Duration::from_millis(1));
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))
}

fn write_server_response(stream: &mut UnixStream, response: &Message) -> io::Result<()> {
    match write_message(stream, response) {
        Ok(()) => Ok(()),
        Err(ProtocolError::FrameTooLarge { .. }) => write_message(
            stream,
            &Message::LibraryOperationFailed(
                "server response exceeded the IPC frame limit; request a smaller catalog page"
                    .to_owned(),
            ),
        )
        .map_err(protocol_error_to_io),
        Err(error) => Err(protocol_error_to_io(error)),
    }
}

fn protocol_error_to_io(error: ProtocolError) -> io::Error {
    match error {
        ProtocolError::Io(error) => error,
        other => io::Error::other(other.to_string()),
    }
}

fn client_disconnected(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    }
}

struct SocketPathCleanup(PathBuf);

impl Drop for SocketPathCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
