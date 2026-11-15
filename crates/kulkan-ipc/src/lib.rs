use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use kulkan_library::{
    BrowsePage, BrowseSelection, BrowseView, CatalogPage, LibraryState, ScanSummary, SearchCriteria,
};
use serde::{Deserialize, Serialize};

/// The current wire-protocol version understood by Kulkán clients and servers.
pub const CURRENT_PROTOCOL_VERSION: u16 = 8;

/// Maximum JSON envelope size accepted on the local socket.
pub const MAX_FRAME_SIZE: usize = 64 * 1024;

/// Commands understood by the current version of the local client/server protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Message {
    Ping,
    Pong,
    Shutdown,
    ShutdownAck,
    Activate,
    ActivateAck,
    AddLibraryRoot(PathBuf),
    RemoveLibraryRoot(PathBuf),
    GetLibraryState,
    GetCatalogPage {
        offset: usize,
        limit: usize,
    },
    BrowseCatalog {
        selection: BrowseSelection,
        offset: usize,
        limit: usize,
    },
    ScanLibrary,
    ScanLibrarySelected(Vec<PathBuf>),
    SetStartupView(BrowseView),
    GetPlayedThreshold,
    SetPlayedThreshold(u8),
    PlayedThreshold(u8),
    ClearEntryPlayHistory(PathBuf),
    ClearAllPlayHistory,
    SearchCatalog {
        criteria: SearchCriteria,
        offset: usize,
        limit: usize,
    },
    ExportAppState(PathBuf),
    ImportAppState(PathBuf),
    ResetAppState,
    AppStateTransferComplete {
        library: LibraryState,
        queue_entries: usize,
        played_threshold_percent: u8,
    },
    LibraryState(LibraryState),
    CatalogPage(CatalogPage),
    BrowsePage(BrowsePage),
    ScanSummary(ScanSummary),
    LibraryOperationFailed(String),
    PlayTrack(PathBuf),
    PausePlayback,
    ResumePlayback,
    SeekPlayback(u64),
    StopPlayback,
    GetPlaybackState,
    PlaybackState(PlaybackSnapshot),
    PlaybackOperationFailed(String),
    AppendQueue(Vec<PathBuf>),
    GetQueueState,
    PlayQueueEntry(u64),
    PreviousQueueEntry,
    SkipQueueEntry,
    ReorderQueueEntry {
        entry_id: u64,
        position: usize,
    },
    RemoveQueueEntry(u64),
    ClearQueue,
    SetRepeatMode(RepeatMode),
    SetShuffle(bool),
    QueueState(QueueSnapshot),
    QueueOperationFailed(String),
}

/// One occurrence of a Track in the listener-managed Queue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueueEntry {
    /// Identifies this occurrence, even when another entry has the same Track path.
    pub id: u64,
    pub track: PathBuf,
    /// A failed entry remains in the Queue but is skipped in later cycles until retried.
    pub failed: bool,
}

/// The repeat behavior for the listener-managed Queue.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

/// Listener-visible queue, ordering settings, and current playback state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueueSnapshot {
    pub entries: Vec<QueueEntry>,
    pub current_entry_id: Option<u64>,
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
    pub playback: PlaybackSnapshot,
    pub play_count_updates: Vec<PlayCountUpdate>,
}

/// A play count that crossed its threshold during the current server update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlayCountUpdate {
    pub track: PathBuf,
    pub play_count: u64,
}

/// Current transport state reported by the child server's GStreamer pipeline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaybackSnapshot {
    pub track: Option<PathBuf>,
    pub status: PlaybackStatus,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// A listener-visible state of the active playback attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    Ended,
    Failed,
}

#[derive(Debug, Deserialize, Serialize)]
struct Envelope {
    version: u16,
    message: Message,
}

/// An error decoding or transporting a protocol message.
#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    InvalidFrame(serde_json::Error),
    UnsupportedVersion(u16),
    FrameTooLarge { length: usize, max: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "local socket I/O failed: {error}"),
            Self::InvalidFrame(error) => write!(f, "invalid IPC frame: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported IPC protocol version {version}")
            }
            Self::FrameTooLarge { length, max } => {
                write!(f, "IPC frame is {length} bytes; maximum is {max}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidFrame(error) => Some(error),
            Self::UnsupportedVersion(_) | Self::FrameTooLarge { .. } => None,
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Binds a Unix socket inside an owner-controlled directory with private permissions.
pub fn bind_private_socket(path: &Path) -> io::Result<UnixListener> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket parent is not a real directory",
        ));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket parent is not owned by the current user",
        ));
    }
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;

    remove_stale_socket(path)?;

    let listener = UnixListener::bind(path)?;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        drop(listener);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(listener)
}

fn remove_stale_socket(path: &Path) -> io::Result<()> {
    let original = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !original.file_type().is_socket() || original.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "socket path exists but is not an owned Unix socket",
        ));
    }

    match UnixStream::connect(path) {
        Ok(stream) => {
            drop(stream);
            Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "a live process owns the socket",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            let current = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(missing) if missing.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(other) => return Err(other),
            };
            if current.file_type().is_socket()
                && current.uid() == original.uid()
                && current.dev() == original.dev()
                && current.ino() == original.ino()
            {
                fs::remove_file(path)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "socket path changed while checking for a live owner",
                ))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Rejects a local socket peer unless it belongs to the current Unix user.
pub fn validate_peer(stream: &UnixStream) -> io::Result<()> {
    let credentials = peer_credentials(stream)?;
    if credentials.uid != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket peer belongs to another Unix user",
        ));
    }
    Ok(())
}

/// Returns the process ID reported by the kernel for a connected Unix socket peer.
pub fn peer_pid(stream: &UnixStream) -> io::Result<u32> {
    u32::try_from(peer_credentials(stream)?.pid).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel returned an invalid peer process ID",
        )
    })
}

fn peer_credentials(stream: &UnixStream) -> io::Result<libc::ucred> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut credentials_len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and credentials_len are valid writable buffers for SO_PEERCRED.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut credentials_len,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if credentials_len as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel returned malformed peer credentials",
        ));
    }
    Ok(credentials)
}

/// Writes one versioned JSON message with a four-byte, big-endian length prefix.
pub fn write_message<W: Write>(writer: &mut W, message: &Message) -> Result<(), ProtocolError> {
    let frame = serde_json::to_vec(&Envelope {
        version: CURRENT_PROTOCOL_VERSION,
        message: message.clone(),
    })
    .map_err(ProtocolError::InvalidFrame)?;

    if frame.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge {
            length: frame.len(),
            max: MAX_FRAME_SIZE,
        });
    }

    let length = u32::try_from(frame.len()).map_err(|_| ProtocolError::FrameTooLarge {
        length: frame.len(),
        max: MAX_FRAME_SIZE,
    })?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&frame)?;
    Ok(())
}

/// Reads exactly one versioned, length-prefixed JSON message.
pub fn read_message<R: Read>(reader: &mut R) -> Result<Message, ProtocolError> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;

    if length > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge {
            length,
            max: MAX_FRAME_SIZE,
        });
    }

    let mut frame = vec![0; length];
    reader.read_exact(&mut frame)?;
    let envelope: Envelope = serde_json::from_slice(&frame).map_err(ProtocolError::InvalidFrame)?;
    if envelope.version != CURRENT_PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.version));
    }
    Ok(envelope.message)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    use crate::{
        CURRENT_PROTOCOL_VERSION, MAX_FRAME_SIZE, Message, ProtocolError, read_message,
        write_message,
    };

    #[test]
    fn messages_round_trip_over_a_unix_stream_with_a_length_prefix() {
        let (mut client, mut server) = UnixStream::pair().unwrap();

        write_message(&mut client, &Message::Ping).unwrap();

        let mut prefix = [0; 4];
        server.read_exact(&mut prefix).unwrap();
        let frame_length = u32::from_be_bytes(prefix) as usize;
        assert!(frame_length > 0);
        assert!(frame_length <= MAX_FRAME_SIZE);
        let mut frame = vec![0; frame_length];
        server.read_exact(&mut frame).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&frame).unwrap()["version"],
            CURRENT_PROTOCOL_VERSION
        );

        let (mut client, mut server) = UnixStream::pair().unwrap();
        write_message(&mut client, &Message::Shutdown).unwrap();
        assert_eq!(read_message(&mut server).unwrap(), Message::Shutdown);
    }

    #[test]
    fn rejects_a_frame_from_an_unsupported_protocol_version() {
        let (mut peer, mut app) = UnixStream::pair().unwrap();
        let frame = br#"{"version":1,"message":"Ping"}"#;
        peer.write_all(&(frame.len() as u32).to_be_bytes()).unwrap();
        peer.write_all(frame).unwrap();

        assert!(matches!(
            read_message(&mut app),
            Err(ProtocolError::UnsupportedVersion(1))
        ));
    }

    #[test]
    fn rejects_an_oversized_frame_before_reading_its_body() {
        let (mut peer, mut app) = UnixStream::pair().unwrap();
        peer.write_all(&((MAX_FRAME_SIZE as u32) + 1).to_be_bytes())
            .unwrap();

        assert!(matches!(
            read_message(&mut app),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }
}
