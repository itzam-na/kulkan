use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kulkan_runtime::ServerProcess;

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kulkan-runtime-library-test-{}-{suffix}",
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
fn adding_a_root_does_not_scan_until_the_listener_requests_a_scan() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    write_tagged_pcm_wav(&root.join("one.wav"));
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();

    let before_scan = server.add_library_root(&root).unwrap();
    assert_eq!(before_scan.selected_roots, vec![root.clone()]);
    assert_eq!(before_scan.catalog_entries, 0);

    let report = server.scan_library().unwrap();
    assert_eq!(report.active_entries, 1);
    let page = server.catalog_page(0, 10).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(page.entries[0].active);
    assert_eq!(server.library_state().unwrap().catalog_entries, 1);

    server.shutdown().unwrap();
}

#[test]
fn an_incomplete_server_scan_preserves_the_last_known_catalog_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    write_tagged_pcm_wav(&root.join("one.wav"));
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();

    server.add_library_root(&root).unwrap();
    let first_scan = server.scan_library().unwrap();
    assert_eq!(first_scan.active_entries, 1);
    fs::remove_dir_all(&root).unwrap();

    let incomplete_scan = server.scan_library().unwrap();

    assert_eq!(incomplete_scan.incomplete_paths, vec![root]);
    assert!(server.catalog_page(0, 10).unwrap().entries[0].active);
    assert_eq!(server.library_state().unwrap().catalog_entries, 1);
    server.shutdown().unwrap();
}

#[test]
fn invalid_catalog_page_requests_are_rejected_without_stopping_the_server() {
    let directory = TestDirectory::new();
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();

    assert!(server.catalog_page(0, 0).is_err());
    assert!(server.is_running().unwrap());
    server.ping().unwrap();
    server.shutdown().unwrap();
}

#[test]
fn catalog_pages_are_bounded_and_continue_in_sorted_path_order() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixture = directory.0.join("fixture.wav");
    write_tagged_pcm_wav(&fixture);
    for index in 0..33 {
        fs::copy(&fixture, root.join(format!("track-{index:02}.wav"))).unwrap();
    }
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();
    server.add_library_root(&root).unwrap();
    assert_eq!(server.scan_library().unwrap().active_entries, 33);

    let first_page = server.catalog_page(0, 32).unwrap();
    let second_page = server
        .catalog_page(first_page.next_offset.unwrap(), 32)
        .unwrap();

    assert_eq!(first_page.entries.len(), 32);
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.next_offset, None);
    assert!(first_page.entries[0].path < first_page.entries[1].path);
    server.shutdown().unwrap();
}

fn write_tagged_pcm_wav(path: &std::path::Path) {
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
    chunk(b"data", &vec![0; 16_000], &mut wav_body);

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(wav_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&wav_body);
    fs::write(path, file).unwrap();
}
