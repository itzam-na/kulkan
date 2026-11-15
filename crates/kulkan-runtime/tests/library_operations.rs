use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
fn large_scan_diagnostic_sets_do_not_overflow_the_ipc_response() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    for index in 0..800 {
        fs::write(
            root.join(format!("broken-{index:04}.m4a")),
            b"invalid audio",
        )
        .unwrap();
    }
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();
    server.add_library_root(&root).unwrap();

    let summary = server.scan_library().unwrap();

    assert_eq!(
        summary.media_errors.len() + summary.omitted_media_errors,
        800
    );
    assert_eq!(
        summary.incomplete_paths.len() + summary.omitted_incomplete_paths,
        800
    );
    assert!(summary.omitted_media_errors > 0);
    server.shutdown().unwrap();
}

#[test]
fn selected_parent_folder_scan_does_not_change_an_unselected_nested_folder() {
    let directory = TestDirectory::new();
    let parent = directory.0.join("music");
    let nested = parent.join("archive");
    fs::create_dir_all(&nested).unwrap();
    let parent_track = parent.join("one.wav");
    let nested_track = nested.join("two.wav");
    write_tagged_pcm_wav(&parent_track);
    write_tagged_pcm_wav(&nested_track);
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server = ServerProcess::start_with_audio_sink(
        &executable,
        &socket_path,
        Duration::from_secs(2),
        Some("fakesink"),
    )
    .unwrap();
    server.add_library_root(&parent).unwrap();
    server.add_library_root(&nested).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();
    server.play_track(&nested_track).unwrap();
    wait_for_play_count(&server, &nested_track, 1);
    server.stop_playback().unwrap();
    let initial_nested_entry = server
        .catalog_page(0, 32)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.path == nested_track)
        .unwrap();
    fs::remove_file(&nested_track).unwrap();

    let scan = server
        .scan_library_selected(std::slice::from_ref(&parent))
        .unwrap();
    let nested_after_scan = server
        .catalog_page(0, 32)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.path == nested_track)
        .unwrap();

    assert_eq!(nested_after_scan, initial_nested_entry);
    assert_eq!(scan.active_entries, 2);
    assert_eq!(scan.inactive_entries, 0);
    server.shutdown().unwrap();
}

fn wait_for_play_count(server: &ServerProcess, track: &Path, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        server.queue_state().unwrap();
        let page = server.catalog_page(0, 32).unwrap();
        if page
            .entries
            .iter()
            .any(|entry| entry.path == track && entry.play_count >= expected)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "play count for {} did not reach {expected}",
            track.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn scan_all_after_removing_the_last_root_marks_entries_inactive_and_keeps_history() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server = ServerProcess::start_with_audio_sink(
        &executable,
        &socket_path,
        Duration::from_secs(2),
        Some("fakesink"),
    )
    .unwrap();
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();
    server.play_track(&track).unwrap();
    wait_for_play_count(&server, &track, 1);
    server.stop_playback().unwrap();
    server.remove_library_root(&root).unwrap();

    let summary = server.scan_library().unwrap();
    let entry = server.catalog_page(0, 32).unwrap().entries.remove(0);

    assert_eq!(summary.active_entries, 0);
    assert_eq!(summary.inactive_entries, 1);
    assert!(!entry.active);
    assert_eq!(entry.play_count, 1);
    server.shutdown().unwrap();
}

#[test]
fn selected_roots_and_scanned_tracks_survive_server_relaunch_without_rescanning() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    write_tagged_pcm_wav(&root.join("one.wav"));
    let socket_path = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));

    let mut first_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    first_server.add_library_root(&root).unwrap();
    assert_eq!(first_server.scan_library().unwrap().active_entries, 1);
    let original_entry = first_server.catalog_page(0, 10).unwrap().entries[0].clone();
    assert_eq!(
        fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(state_path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    first_server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();

    let restored_state = restored_server.library_state().unwrap();
    let restored_entry = restored_server
        .catalog_page(0, 10)
        .unwrap()
        .entries
        .remove(0);
    assert_eq!(restored_state.selected_roots, vec![root]);
    assert_eq!(restored_state.catalog_entries, 1);
    assert_eq!(restored_entry, original_entry);
    restored_server.shutdown().unwrap();
}

#[test]
fn catalog_browsing_and_startup_view_work_through_the_child_server() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    write_tagged_pcm_wav(&root.join("one.wav"));
    write_tagged_pcm_wav(&root.join("two.wav"));
    let socket_path = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server
        .set_startup_view(kulkan_library::BrowseView::Artists)
        .unwrap();

    let artists = server
        .browse_catalog(
            kulkan_library::BrowseSelection::View(kulkan_library::BrowseView::Artists),
            0,
            10,
        )
        .unwrap();
    assert_eq!(
        artists.items,
        vec![kulkan_library::BrowseItem::Artist {
            name: None,
            track_count: 2,
        }]
    );
    let albums = server
        .browse_catalog(
            kulkan_library::BrowseSelection::View(kulkan_library::BrowseView::Albums),
            0,
            10,
        )
        .unwrap();
    assert_eq!(
        albums.items,
        vec![kulkan_library::BrowseItem::Album {
            key: kulkan_library::AlbumKey::NoAlbum,
            track_count: 2,
        }]
    );
    let songs = server
        .browse_catalog(
            kulkan_library::BrowseSelection::View(kulkan_library::BrowseView::Songs),
            0,
            10,
        )
        .unwrap();
    assert_eq!(songs.items.len(), 2);
    assert!(songs.items.iter().all(|item| matches!(
        item,
        kulkan_library::BrowseItem::Track(entry) if entry.active
    )));
    fs::create_dir(directory.0.join("backup")).unwrap();
    let export_path = directory.0.join("backup/app-state.json");
    server.export_app_state(&export_path).unwrap();
    server
        .set_startup_view(kulkan_library::BrowseView::Songs)
        .unwrap();
    assert_eq!(
        server.import_app_state(&export_path).unwrap().startup_view,
        kulkan_library::BrowseView::Artists
    );
    server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        restored_server.library_state().unwrap().startup_view,
        kulkan_library::BrowseView::Artists
    );
    restored_server.shutdown().unwrap();
}

#[test]
fn a_selected_root_survives_relaunch_before_its_first_scan() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    write_tagged_pcm_wav(&root.join("one.wav"));
    let socket_path = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));

    let mut first_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    first_server.add_library_root(&root).unwrap();
    first_server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    let restored_state = restored_server.library_state().unwrap();

    assert_eq!(restored_state.selected_roots, vec![root.clone()]);
    assert_eq!(restored_state.catalog_entries, 0);
    assert_eq!(restored_server.scan_library().unwrap().active_entries, 1);
    restored_server.shutdown().unwrap();
}

#[test]
fn played_threshold_defaults_to_seventy_five_persists_and_rejects_out_of_range_values() {
    let directory = TestDirectory::new();
    let socket_path = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut first_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();

    assert_eq!(first_server.played_threshold().unwrap(), 75);
    first_server.set_played_threshold(1).unwrap();
    assert!(first_server.set_played_threshold(0).is_err());
    assert!(first_server.set_played_threshold(101).is_err());
    first_server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(restored_server.played_threshold().unwrap(), 1);
    restored_server.shutdown().unwrap();
}

#[test]
fn previous_library_state_versions_restore_with_the_default_threshold() {
    let directory = TestDirectory::new();
    let socket_path = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(
        &state_path,
        br#"{"version":1,"selected_roots":[],"catalog":{"roots":[],"entries":[]}}"#,
    )
    .unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));

    let mut server = ServerProcess::start_with_library_state(
        &executable,
        &socket_path,
        &state_path,
        Duration::from_secs(2),
    )
    .unwrap();

    assert_eq!(server.played_threshold().unwrap(), 75);
    server.shutdown().unwrap();
}

#[test]
fn server_search_returns_filtered_bounded_pages_and_rejects_invalid_ranges() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kulkan-library/tests/fixtures/metadata.mp3");
    for name in ["track-00.mp3", "track-01.mp3", "other.mp3"] {
        fs::copy(&fixture, root.join(name)).unwrap();
    }
    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();

    let criteria = kulkan_runtime::SearchCriteria {
        file_location: Some("TRACK-".to_owned()),
        ..kulkan_runtime::SearchCriteria::default()
    };
    let first_page = server.search_catalog(&criteria, 0, 1).unwrap();
    assert_eq!(first_page.entries.len(), 1);
    assert_eq!(first_page.next_offset, Some(1));
    let second_page = server
        .search_catalog(&criteria, first_page.next_offset.unwrap(), 1)
        .unwrap();
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.next_offset, None);
    assert!(first_page.entries[0].path < second_page.entries[0].path);

    let invalid = kulkan_runtime::SearchCriteria {
        duration_seconds: Some(kulkan_library::NumericFilter::IsMissing),
        ..kulkan_runtime::SearchCriteria::default()
    };
    assert!(server.search_catalog(&invalid, 0, 10).is_err());
    server.shutdown().unwrap();
}

#[test]
fn unfiltered_server_search_returns_inactive_entries_in_library_order_across_pages() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kulkan-library/tests/fixtures");
    let ordinary_track = fixtures.join("metadata.mp3");
    let earlier_artist = fixtures.join("metadata-duplicate-artists.mp3");
    let mut expected_paths = Vec::new();
    for index in 0..33 {
        let path = root.join(format!("track-{index:02}.mp3"));
        fs::copy(&ordinary_track, &path).unwrap();
        expected_paths.push(path);
    }
    let artist_first_path = root.join("artist-first.mp3");
    fs::copy(earlier_artist, &artist_first_path).unwrap();
    expected_paths.push(artist_first_path.clone());

    let socket_path = directory.0.join("server.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server =
        ServerProcess::start(&executable, &socket_path, Duration::from_secs(2)).unwrap();
    server.add_library_root(&root).unwrap();
    assert_eq!(server.scan_library().unwrap().active_entries, 34);

    let inactive_path = root.join("track-05.mp3");
    fs::remove_file(&inactive_path).unwrap();
    let rescan = server.scan_library().unwrap();
    assert_eq!(rescan.active_entries, 33);
    assert_eq!(rescan.inactive_entries, 1);

    let first_page = server
        .search_catalog(&kulkan_runtime::SearchCriteria::default(), 0, 32)
        .unwrap();
    let second_page = server
        .search_catalog(
            &kulkan_runtime::SearchCriteria::default(),
            first_page.next_offset.unwrap(),
            32,
        )
        .unwrap();

    assert_eq!(first_page.entries.len(), 32);
    assert_eq!(first_page.next_offset, Some(32));
    assert_eq!(second_page.entries.len(), 2);
    assert_eq!(second_page.next_offset, None);
    assert_eq!(first_page.entries[0].path, artist_first_path);
    assert!(
        first_page
            .entries
            .iter()
            .any(|entry| entry.path == inactive_path && !entry.active)
    );

    let actual_paths = first_page
        .entries
        .iter()
        .chain(&second_page.entries)
        .map(|entry| entry.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual_paths.len(), 34);
    assert_eq!(actual_paths, expected_paths.into_iter().collect());
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
