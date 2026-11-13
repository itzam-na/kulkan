use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use kulkan_library::{Catalog, scan_library};

static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kulkan-library-test-{}-{suffix}",
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

#[cfg(unix)]
#[test]
fn overlapping_roots_keep_each_resolved_audio_path_once_and_skip_nested_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    let album = root.join("album");
    let external = directory.0.join("outside");
    let selected_root = directory.0.join("selected-root");
    fs::create_dir_all(&album).unwrap();
    fs::create_dir(&external).unwrap();
    fs::create_dir(&selected_root).unwrap();

    let first_track = album.join("one.wav");
    let nested_symlink = album.join("outside.mp3");
    let explicitly_selected_track = selected_root.join("selected.wav");
    write_tagged_pcm_wav(&first_track);
    fs::write(external.join("outside.mp3"), b"outside fixture").unwrap();
    write_tagged_pcm_wav(&explicitly_selected_track);
    fs::write(album.join("notes.txt"), b"not audio").unwrap();
    symlink(external.join("outside.mp3"), &nested_symlink).unwrap();

    let explicit_directory_link = directory.0.join("selected-library");
    symlink(&selected_root, &explicit_directory_link).unwrap();

    let before = fs::read(&first_track).unwrap();
    let result = scan_library(
        &Catalog::default(),
        &[root.clone(), album, explicit_directory_link],
    );

    let mut actual: Vec<_> = result
        .catalog
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    actual.sort();
    let mut expected = vec![first_track.clone(), explicitly_selected_track];
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(fs::read(&first_track).unwrap(), before);
}

#[test]
fn a_root_that_becomes_unavailable_does_not_mark_its_last_known_tracks_missing() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);

    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    assert_eq!(previous.entries.len(), 1);
    fs::remove_dir_all(&root).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert!(result.catalog.entries[0].active);
    assert_eq!(result.catalog.entries[0].path, track);
    assert_eq!(result.incomplete_paths, vec![root]);
}

#[test]
fn removing_a_root_after_an_incomplete_scan_still_deactivates_its_last_known_entries() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);
    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    fs::remove_dir_all(&root).unwrap();

    let unavailable = scan_library(&previous, std::slice::from_ref(&root));
    let removed = scan_library(&unavailable.catalog, &[]);

    assert!(unavailable.catalog.entries[0].active);
    assert!(!removed.catalog.entries[0].active);
}

#[cfg(unix)]
#[test]
fn an_unavailable_overlapping_root_preserves_entries_when_another_root_is_removed() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let target = directory.0.join("music");
    fs::create_dir(&target).unwrap();
    write_tagged_pcm_wav(&target.join("one.wav"));
    let first_root = directory.0.join("first-root");
    let remaining_root = directory.0.join("remaining-root");
    symlink(&target, &first_root).unwrap();
    symlink(&target, &remaining_root).unwrap();
    let previous = scan_library(
        &Catalog::default(),
        &[first_root.clone(), remaining_root.clone()],
    )
    .catalog;
    fs::remove_dir_all(&target).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&remaining_root));

    assert!(result.catalog.entries[0].active);
    assert!(result.incomplete_paths.contains(&remaining_root));
}

#[cfg(unix)]
#[test]
fn entries_under_nested_symlinks_are_inactive_after_the_real_directory_is_replaced() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    let nested = root.join("album");
    let external = directory.0.join("external");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(&external).unwrap();
    let old_track = nested.join("one.wav");
    write_tagged_pcm_wav(&old_track);
    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;

    fs::remove_dir_all(&nested).unwrap();
    write_tagged_pcm_wav(&external.join("one.wav"));
    symlink(&external, &nested).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert!(!result.catalog.entries[0].active);
    assert!(result.incomplete_paths.is_empty());
}

#[test]
fn catalog_keeps_original_wav_tags_and_their_format_specific_provenance() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty());
    let metadata = &result.catalog.entries[0].metadata;
    assert_eq!(
        metadata
            .artists
            .iter()
            .map(|credit| credit.value.as_str())
            .collect::<Vec<_>>(),
        ["Ana & Ben"]
    );
    assert_eq!(
        metadata
            .release_title
            .as_ref()
            .map(|tag| tag.value.as_str()),
        Some("Live set")
    );
    assert_eq!(metadata.genres[0].value, "Ambient music");
    assert_eq!(metadata.genres[0].field, "IGNR");
    assert_eq!(metadata.release_date, None);
    assert_eq!(metadata.release_year, None);
    assert_eq!(
        result.catalog.entries[0].duration,
        Some(std::time::Duration::from_secs(1))
    );
}

#[test]
fn supported_formats_keep_original_tags_and_do_not_promote_recording_dates_to_releases() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();

    for extension in ["mp3", "wav", "flac", "ogg", "opus"] {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("metadata.{extension}"));
        fs::copy(&fixture, root.join(format!("metadata.{extension}"))).unwrap();
    }

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 5);
    for entry in &result.catalog.entries {
        let metadata = &entry.metadata;
        assert_eq!(
            metadata.artists[0].value,
            "Ana & Ben",
            "{}",
            entry.path.display()
        );
        assert_eq!(metadata.release_title.as_ref().unwrap().value, "Live set");
        assert_eq!(metadata.genres[0].value, "Ambient / electronic");
        assert_eq!(metadata.track_number, Some(2));
        assert_eq!(metadata.track_total, Some(7));
        assert_eq!(metadata.release_date, None);
        assert_eq!(metadata.release_year, None);
    }
}

#[test]
fn only_valid_explicit_release_dates_supply_a_release_year() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    for fixture_name in ["release-date.flac", "invalid-release-date.flac"] {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture_name);
        fs::copy(fixture, root.join(fixture_name)).unwrap();
    }

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    let valid = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path.ends_with("release-date.flac"))
        .unwrap();
    assert_eq!(
        valid.metadata.release_date.as_ref().unwrap().value,
        "1999-02"
    );
    assert_eq!(valid.metadata.release_year, Some(1999));
    let invalid = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path.ends_with("invalid-release-date.flac"))
        .unwrap();
    assert_eq!(invalid.metadata.release_date, None);
    assert_eq!(invalid.metadata.release_year, None);
    assert!(
        invalid
            .metadata
            .conflicting_values
            .iter()
            .any(|value| value.value == "2024-13")
    );
}

#[test]
fn id3v2_genre_precedes_and_retains_conflicting_id3v1_genre_provenance() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/metadata-id3v2-precedence.mp3");
    fs::copy(fixture, root.join("metadata.mp3")).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    let metadata = &result.catalog.entries[0].metadata;
    assert_eq!(metadata.genres[0].value, "Ambient / electronic");
    assert!(
        metadata
            .conflicting_values
            .iter()
            .any(|value| { value.source == "Id3v1" && value.value == "Pop" })
    );
}

#[test]
fn repeated_vorbis_genre_fields_keep_their_order_without_splitting_punctuation() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata-multi-genre.flac");
    fs::copy(fixture, root.join("metadata.flac")).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    let genres = &result.catalog.entries[0].metadata.genres;
    assert_eq!(
        genres
            .iter()
            .map(|genre| genre.value.as_str())
            .collect::<Vec<_>>(),
        ["Ambient", "Electronic / dance"]
    );
    assert!(genres.iter().all(|genre| genre.source == "VorbisComments"));
}

#[test]
fn duplicate_artist_credits_are_removed_without_changing_first_billed_order() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/metadata-duplicate-artists.mp3");
    fs::copy(fixture, root.join("metadata.mp3")).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(
        result.catalog.entries[0]
            .metadata
            .artists
            .iter()
            .map(|artist| artist.value.as_str())
            .collect::<Vec<_>>(),
        ["Ana", "Ben"]
    );
}

#[test]
fn a_malformed_audio_file_is_reported_and_does_not_destroy_its_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);
    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    fs::write(&track, b"not a valid WAVE file").unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.media_errors[0].path, track);
    assert!(!result.media_errors[0].message.is_empty());
    assert!(result.catalog.entries[0].active);
    assert_eq!(
        result.catalog.entries[0].metadata.artists[0].value,
        "Ana & Ben"
    );
}

#[test]
fn successfully_missing_files_and_removed_roots_become_inactive() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav(&track);
    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;

    fs::remove_file(&track).unwrap();
    let after_file_removal = scan_library(&previous, std::slice::from_ref(&root));
    assert!(!after_file_removal.catalog.entries[0].active);
    assert!(after_file_removal.incomplete_paths.is_empty());

    let after_root_removal = scan_library(&previous, &[]);
    assert!(!after_root_removal.catalog.entries[0].active);
}

#[cfg(unix)]
#[test]
fn an_unreadable_subtree_is_reported_without_marking_its_entries_missing() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    let unreadable = root.join("private");
    fs::create_dir_all(&unreadable).unwrap();
    let track = unreadable.join("one.wav");
    write_tagged_pcm_wav(&track);
    let previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;

    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&unreadable).is_ok() {
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }

    let result = scan_library(&previous, std::slice::from_ref(&root));
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(result.incomplete_paths.contains(&unreadable));
    assert!(result.catalog.entries[0].active);
    assert_eq!(result.catalog.entries[0].path, track);
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

    fn info_item(id: &[u8; 4], value: &str, destination: &mut Vec<u8>) {
        let mut text = value.as_bytes().to_vec();
        text.push(0);
        chunk(id, &text, destination);
    }

    let mut wav_body = b"WAVE".to_vec();
    let mut format = Vec::new();
    format.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    format.extend_from_slice(&1_u16.to_le_bytes()); // mono
    format.extend_from_slice(&8_000_u32.to_le_bytes());
    format.extend_from_slice(&16_000_u32.to_le_bytes());
    format.extend_from_slice(&2_u16.to_le_bytes());
    format.extend_from_slice(&16_u16.to_le_bytes());
    chunk(b"fmt ", &format, &mut wav_body);

    let mut info = b"INFO".to_vec();
    info_item(b"IART", "Ana & Ben", &mut info);
    info_item(b"IPRD", "Live set", &mut info);
    info_item(b"IGNR", "Ambient music", &mut info);
    info_item(b"ICRD", "2000-01-01", &mut info);
    chunk(b"LIST", &info, &mut wav_body);
    chunk(b"data", &vec![0; 16_000], &mut wav_body);

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(wav_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&wav_body);
    fs::write(path, file).unwrap();
}
