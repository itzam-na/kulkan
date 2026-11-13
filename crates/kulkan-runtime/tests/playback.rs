use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kulkan_ipc::PlaybackStatus;
use kulkan_runtime::ServerProcess;

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kulkan-runtime-playback-test-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn server_playback_controls_play_pause_resume_seek_and_stop() {
    let directory = TestDirectory::new();
    let track = directory.0.join("ten-seconds.wav");
    write_pcm_wav(&track, 80_000);
    let mut server = start_headless_server(&directory);

    let started = server.play_track(&track).unwrap();
    assert_eq!(started.track, Some(track.clone()));
    assert_eq!(started.status, PlaybackStatus::Playing);
    let with_duration = wait_until(&server, |state| state.duration_ms.is_some());
    assert_eq!(with_duration.duration_ms, Some(10_000));

    let paused = server.pause_playback().unwrap();
    assert_eq!(paused.status, PlaybackStatus::Paused);
    thread::sleep(Duration::from_millis(80));
    let still_paused = server.playback_state().unwrap();
    assert_eq!(still_paused.status, PlaybackStatus::Paused);
    assert!(still_paused.position_ms <= paused.position_ms + 5);

    let resumed = server.resume_playback().unwrap();
    assert_eq!(resumed.status, PlaybackStatus::Playing);
    let sought = server.seek_playback(5_000).unwrap();
    assert_eq!(sought.status, PlaybackStatus::Playing);
    let seek_position = wait_for_position(&server, 4_900);
    assert!(seek_position.position_ms >= 4_900);

    let stopped = server.stop_playback().unwrap();
    assert_eq!(stopped.status, PlaybackStatus::Stopped);
    assert_eq!(stopped.track, None);
    server.shutdown().unwrap();
}

#[test]
fn every_v1_audio_format_decodes_through_the_child_server() {
    let directory = TestDirectory::new();
    let mut server = start_headless_server(&directory);
    let fixture_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kulkan-library/tests/fixtures");

    for extension in ["mp3", "wav", "flac", "ogg", "opus"] {
        let track = directory.0.join(format!("track.{extension}"));
        fs::copy(fixture_dir.join(format!("metadata.{extension}")), &track).unwrap();
        let started = server.play_track(&track).unwrap();
        assert_eq!(started.track, Some(track.clone()));
        let finished = wait_for_status(&server, PlaybackStatus::Ended);
        assert_eq!(finished.track, Some(track), "{extension}");
        assert!(
            finished.error.is_none(),
            "{extension}: {:?}",
            finished.error
        );
    }

    server.shutdown().unwrap();
}

#[test]
fn unreadable_media_reports_an_actionable_playback_error() {
    let directory = TestDirectory::new();
    let track = directory.0.join("broken.wav");
    fs::write(&track, b"not a WAVE file").unwrap();
    let mut server = start_headless_server(&directory);

    server.play_track(&track).unwrap();
    let failed = wait_for_status(&server, PlaybackStatus::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| { error.contains("GStreamer") || error.contains("audio") })
    );
    server.shutdown().unwrap();
}

#[test]
fn unsupported_aac_m4a_media_reports_an_actionable_playback_error() {
    let directory = TestDirectory::new();
    let track = directory.0.join("unsupported.m4a");
    fs::write(&track, b"AAC/M4A is not supported in this version").unwrap();
    let mut server = start_headless_server(&directory);

    let error = server.play_track(&track).unwrap_err();

    assert!(error.to_string().contains("AAC/M4A"));
    assert!(error.to_string().contains("not supported"));
    server.shutdown().unwrap();
}

fn start_headless_server(directory: &TestDirectory) -> ServerProcess {
    let socket = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    ServerProcess::start_with_audio_sink(
        &executable,
        &socket,
        Duration::from_secs(5),
        Some("fakesink"),
    )
    .unwrap()
}

fn wait_for_position(server: &ServerProcess, minimum_ms: u64) -> kulkan_ipc::PlaybackSnapshot {
    wait_until(server, |state| state.position_ms >= minimum_ms)
}

fn wait_for_status(
    server: &ServerProcess,
    expected: PlaybackStatus,
) -> kulkan_ipc::PlaybackSnapshot {
    wait_until(server, |state| {
        state.status == expected || state.status == PlaybackStatus::Failed
    })
}

fn wait_until(
    server: &ServerProcess,
    predicate: impl Fn(&kulkan_ipc::PlaybackSnapshot) -> bool,
) -> kulkan_ipc::PlaybackSnapshot {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = server.playback_state().unwrap();
        if predicate(&state) || state.status == PlaybackStatus::Failed {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "playback did not reach the expected state: {state:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn write_pcm_wav(path: &std::path::Path, frames: usize) {
    fn chunk(id: &[u8; 4], body: &[u8], destination: &mut Vec<u8>) {
        destination.extend_from_slice(id);
        destination.extend_from_slice(&(body.len() as u32).to_le_bytes());
        destination.extend_from_slice(body);
        if body.len() % 2 != 0 {
            destination.push(0);
        }
    }

    let mut wav_body = b"WAVE".to_vec();
    let mut format = Vec::new();
    format.extend_from_slice(&1_u16.to_le_bytes());
    format.extend_from_slice(&1_u16.to_le_bytes());
    format.extend_from_slice(&8_000_u32.to_le_bytes());
    format.extend_from_slice(&16_000_u32.to_le_bytes());
    format.extend_from_slice(&2_u16.to_le_bytes());
    format.extend_from_slice(&16_u16.to_le_bytes());
    chunk(b"fmt ", &format, &mut wav_body);
    chunk(b"data", &vec![0; frames * 2], &mut wav_body);
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(wav_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&wav_body);
    fs::write(path, file).unwrap();
}
