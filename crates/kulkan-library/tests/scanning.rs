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
fn metadata_only_changes_preserve_library_entry_identity_and_play_count() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav_with_album(&track, "Live set");
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let previous_id = previous.entries[0].id;

    write_tagged_pcm_wav_with_album(&track, "Studio set");
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, previous_id);
    assert_eq!(result.catalog.entries[0].play_count, 7);
    assert_eq!(
        result.catalog.entries[0]
            .metadata
            .release_title
            .as_ref()
            .unwrap()
            .value,
        "Studio set"
    );
}

#[test]
fn metadata_only_changes_preserve_identity_in_every_supported_format() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    for extension in ["mp3", "flac", "wav", "ogg", "opus"] {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("metadata.{extension}"));
        fs::copy(fixture, root.join(format!("track.{extension}"))).unwrap();
    }
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    assert_eq!(previous.entries.len(), 5);
    for entry in &mut previous.entries {
        entry.play_count = 7;
    }
    let ids = previous
        .entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.id))
        .collect::<std::collections::HashMap<_, _>>();
    for entry in &previous.entries {
        replace_bytes_once(&entry.path, b"Live set", b"Live SET");
    }

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 5);
    for entry in &result.catalog.entries {
        assert_eq!(entry.id, ids[&entry.path], "{}", entry.path.display());
        assert_eq!(entry.play_count, 7, "{}", entry.path.display());
        assert_eq!(
            entry.metadata.release_title.as_ref().unwrap().value,
            "Live SET",
            "{}",
            entry.path.display()
        );
    }
}

#[test]
fn changing_opus_output_gain_changes_the_content_identity() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.opus");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.opus");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    let head = bytes
        .windows(b"OpusHead".len())
        .position(|window| window == b"OpusHead")
        .unwrap();
    bytes[head + 16] = bytes[head + 16].wrapping_add(1);
    refresh_ogg_checksums(&mut bytes);
    fs::write(&track, bytes).unwrap();
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 2);
    let current = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.active)
        .unwrap();
    assert_ne!(current.id, old_id);
    assert_eq!(current.play_count, 0);
    assert!(
        result
            .catalog
            .entries
            .iter()
            .any(|entry| entry.id == old_id && !entry.active && entry.play_count == 3)
    );
}

#[test]
fn a_truncated_ogg_packet_does_not_replace_the_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.ogg");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.ogg");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    bytes.pop();
    fs::write(&track, bytes).unwrap();
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, old_id);
    assert_eq!(result.catalog.entries[0].play_count, 3);
    assert!(result.catalog.entries[0].active);
}

#[test]
fn a_missing_ogg_end_page_does_not_replace_the_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.ogg");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.ogg");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    let last_page = bytes
        .windows(4)
        .rposition(|window| window == b"OggS")
        .unwrap();
    let lacing_count = bytes[last_page + 26] as usize;
    let body_size = bytes[last_page + 27..last_page + 27 + lacing_count]
        .iter()
        .map(|size| usize::from(*size))
        .sum::<usize>();
    assert_eq!(last_page + 27 + lacing_count + body_size, bytes.len());
    bytes.truncate(last_page);
    fs::write(&track, bytes).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, old_id);
    assert_eq!(result.catalog.entries[0].play_count, 3);
    assert!(result.catalog.entries[0].active);
}

#[test]
fn a_corrupt_ogg_page_does_not_replace_the_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.ogg");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.ogg");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    let last_page = bytes
        .windows(4)
        .rposition(|window| window == b"OggS")
        .unwrap();
    let lacing_count = bytes[last_page + 26] as usize;
    let body_start = last_page + 27 + lacing_count;
    let body_size = bytes[last_page + 27..body_start]
        .iter()
        .map(|size| usize::from(*size))
        .sum::<usize>();
    assert!(body_size > 0);
    bytes[body_start + body_size - 1] ^= 1;
    fs::write(&track, bytes).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, old_id);
    assert_eq!(result.catalog.entries[0].play_count, 3);
    assert!(result.catalog.entries[0].active);
}

#[test]
fn chained_ogg_streams_of_the_same_codec_are_fingerprinted_together() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("chained.ogg");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.ogg");
    let first = fs::read(&fixture).unwrap();
    let mut second = fs::read(fixture).unwrap();
    let first_serial = u32::from_le_bytes(first[14..18].try_into().unwrap());
    rewrite_ogg_serial(&mut second, first_serial.wrapping_add(1));
    let mut chained = first;
    chained.extend_from_slice(&second);
    fs::write(&track, chained).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 1);
    assert!(result.catalog.entries[0].active);
}

#[test]
fn changed_audio_at_the_same_path_gets_new_identity_and_keeps_old_history_inactive() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav_with_sample(&track, "Live set", 0);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let old_id = previous.entries[0].id;

    write_tagged_pcm_wav_with_sample(&track, "Live set", 1);
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 2);
    let current = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.active)
        .unwrap();
    let old = result
        .catalog
        .entries
        .iter()
        .find(|entry| !entry.active)
        .unwrap();
    assert_ne!(current.id, old_id);
    assert_eq!(current.play_count, 0);
    assert_eq!(old.id, old_id);
    assert_eq!(old.play_count, 7);
}

#[test]
fn changed_content_at_a_path_never_borrows_another_inactive_entries_history() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let existing_path = root.join("a-existing.wav");
    let other_path = root.join("z-other.wav");
    write_tagged_pcm_wav_with_sample(&existing_path, "Live set", 0);
    write_tagged_pcm_wav_with_sample(&other_path, "Live set", 1);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    let existing_id = previous
        .entries
        .iter()
        .find(|entry| entry.path == existing_path)
        .unwrap()
        .id;
    let other = previous
        .entries
        .iter_mut()
        .find(|entry| entry.path == other_path)
        .unwrap();
    other.play_count = 9;
    let other_id = other.id;
    fs::remove_file(&other_path).unwrap();
    write_tagged_pcm_wav_with_sample(&existing_path, "Live set", 1);

    let result = scan_library(&previous, std::slice::from_ref(&root));

    let current = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path == existing_path && entry.active)
        .unwrap();
    assert_ne!(current.id, existing_id);
    assert_ne!(current.id, other_id);
    assert_eq!(current.play_count, 0);
    assert!(
        result
            .catalog
            .entries
            .iter()
            .any(|entry| entry.id == other_id && !entry.active && entry.play_count == 9)
    );
}

#[test]
fn returning_audio_at_its_original_path_restores_its_inactive_identity_and_history() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("one.wav");
    write_tagged_pcm_wav_with_sample(&track, "Live set", 0);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;

    fs::remove_file(&track).unwrap();
    let missing = scan_library(&previous, std::slice::from_ref(&root));
    assert!(!missing.catalog.entries[0].active);
    write_tagged_pcm_wav_with_sample(&track, "Studio set", 0);
    let returned = scan_library(&missing.catalog, std::slice::from_ref(&root));

    assert_eq!(returned.catalog.entries.len(), 1);
    assert_eq!(returned.catalog.entries[0].id, original_id);
    assert_eq!(returned.catalog.entries[0].play_count, 7);
    assert!(returned.catalog.entries[0].active);
    assert_eq!(
        returned.catalog.entries[0]
            .metadata
            .release_title
            .as_ref()
            .unwrap()
            .value,
        "Studio set"
    );
}

#[test]
fn a_unique_content_match_at_a_new_path_transfers_identity_and_play_count() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let old_path = root.join("old.wav");
    let new_path = root.join("new.wav");
    write_tagged_pcm_wav(&old_path);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;
    fs::rename(&old_path, &new_path).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].path, new_path);
    assert_eq!(result.catalog.entries[0].id, original_id);
    assert_eq!(result.catalog.entries[0].play_count, 7);
}

#[test]
fn a_reencoded_track_at_a_new_path_does_not_inherit_history() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let old_path = root.join("old.wav");
    let new_path = root.join("new.wav");
    write_tagged_pcm_wav_with_sample(&old_path, "Live set", 0);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;
    fs::remove_file(&old_path).unwrap();
    write_tagged_pcm_wav_with_sample(&new_path, "Live set", 1);

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 2);
    let current = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path == new_path)
        .unwrap();
    assert!(current.active);
    assert_ne!(current.id, original_id);
    assert_eq!(current.play_count, 0);
    assert!(
        result
            .catalog
            .entries
            .iter()
            .any(|entry| entry.id == original_id && !entry.active && entry.play_count == 7)
    );
}

#[test]
fn ambiguous_content_matches_do_not_transfer_history_to_a_new_path() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    write_tagged_pcm_wav(&first);
    fs::copy(&first, &second).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    previous.entries[1].play_count = 5;
    fs::remove_file(&first).unwrap();
    fs::remove_file(&second).unwrap();
    let moved = root.join("moved.wav");
    write_tagged_pcm_wav(&moved);

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 3);
    let new_entry = result
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path == moved)
        .unwrap();
    assert_eq!(new_entry.play_count, 0);
    assert!(previous.entries.iter().all(|old| {
        result
            .catalog
            .entries
            .iter()
            .any(|entry| entry.id == old.id && !entry.active)
    }));
}

#[test]
fn identical_audio_at_distinct_paths_remains_two_active_entries() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    write_tagged_pcm_wav(&first);
    fs::copy(&first, &second).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 2);
    assert!(result.catalog.entries.iter().all(|entry| entry.active));
    assert_ne!(result.catalog.entries[0].id, result.catalog.entries[1].id);
}

#[test]
fn an_old_entry_does_not_choose_between_two_new_paths_with_matching_audio() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let original = root.join("z-original.wav");
    write_tagged_pcm_wav(&original);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;
    fs::remove_file(&original).unwrap();
    let first_scan = scan_library(&previous, std::slice::from_ref(&root));
    let first_copy = root.join("a-copy.wav");
    let second_copy = root.join("b-copy.wav");
    write_tagged_pcm_wav(&first_copy);
    fs::copy(&first_copy, &second_copy).unwrap();

    let result = scan_library(&first_scan.catalog, std::slice::from_ref(&root));

    assert_eq!(result.catalog.entries.len(), 3);
    assert!(
        result
            .catalog
            .entries
            .iter()
            .any(|entry| entry.id == original_id && !entry.active && entry.play_count == 7)
    );
    for path in [&first_copy, &second_copy] {
        let entry = result
            .catalog
            .entries
            .iter()
            .find(|entry| entry.path == *path)
            .unwrap();
        assert!(entry.active);
        assert_eq!(entry.play_count, 0);
        assert_ne!(entry.id, original_id);
    }
}

#[test]
fn valid_waveformat_extensible_pcm_is_fingerprinted() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("extensible.wav");
    write_tagged_pcm_wav(&track);

    let mut bytes = fs::read(&track).unwrap();
    let fmt = bytes.windows(4).position(|chunk| chunk == b"fmt ").unwrap();
    let old_size = u32::from_le_bytes(bytes[fmt + 4..fmt + 8].try_into().unwrap());
    assert_eq!(old_size, 16);
    bytes[fmt + 4..fmt + 8].copy_from_slice(&40_u32.to_le_bytes());
    bytes[fmt + 8..fmt + 10].copy_from_slice(&0xfffe_u16.to_le_bytes());
    let extension = [
        22, 0, // extension size
        16, 0, // valid bits per sample
        4, 0, 0, 0, // mono center channel mask
        1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
    ];
    bytes.splice(fmt + 24..fmt + 24, extension);
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) + 24;
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    fs::write(&track, bytes).unwrap();

    let result = scan_library(&Catalog::default(), std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 1);
}

#[test]
fn an_apev2_tag_edit_does_not_change_mp3_identity() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.mp3");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.mp3");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;
    let mut ape_footer = b"APETAGEX".to_vec();
    ape_footer.extend_from_slice(&2_000_u32.to_le_bytes());
    ape_footer.extend_from_slice(&32_u32.to_le_bytes());
    ape_footer.extend_from_slice(&0_u32.to_le_bytes());
    ape_footer.extend_from_slice(&0_u32.to_le_bytes());
    ape_footer.extend_from_slice(&[0; 8]);
    let mut bytes = fs::read(&track).unwrap();
    bytes.extend_from_slice(&ape_footer);
    fs::write(&track, bytes).unwrap();

    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, original_id);
    assert_eq!(result.catalog.entries[0].play_count, 7);
}

#[test]
fn a_leading_id3_tag_does_not_change_flac_identity() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.flac");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.flac");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;

    let mut bytes = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    bytes.extend_from_slice(&fs::read(&track).unwrap());
    fs::write(&track, bytes).unwrap();
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert!(result.media_errors.is_empty(), "{:?}", result.media_errors);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, original_id);
    assert_eq!(result.catalog.entries[0].play_count, 7);
}

#[test]
fn returning_entry_keeps_its_identity_when_an_earlier_sorted_copy_is_added() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let original = root.join("z-original.wav");
    write_tagged_pcm_wav(&original);
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 7;
    let original_id = previous.entries[0].id;

    fs::remove_file(&original).unwrap();
    let missing = scan_library(&previous, std::slice::from_ref(&root));
    let copy = root.join("a-copy.wav");
    write_tagged_pcm_wav(&original);
    fs::copy(&original, &copy).unwrap();
    let returned = scan_library(&missing.catalog, std::slice::from_ref(&root));

    let original_entry = returned
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path == original)
        .unwrap();
    let copy_entry = returned
        .catalog
        .entries
        .iter()
        .find(|entry| entry.path == copy)
        .unwrap();
    assert_eq!(original_entry.id, original_id);
    assert_eq!(original_entry.play_count, 7);
    assert_ne!(copy_entry.id, original_id);
    assert_eq!(copy_entry.play_count, 0);
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
fn a_truncated_flac_frame_does_not_replace_the_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.flac");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.flac");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    bytes.pop();
    fs::write(&track, bytes).unwrap();
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, old_id);
    assert_eq!(result.catalog.entries[0].play_count, 3);
    assert!(result.catalog.entries[0].active);
}

#[test]
fn flac_streaminfo_sample_count_mismatch_does_not_replace_the_last_known_entry() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("track.flac");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.flac");
    fs::copy(fixture, &track).unwrap();
    let mut previous = scan_library(&Catalog::default(), std::slice::from_ref(&root)).catalog;
    previous.entries[0].play_count = 3;
    let old_id = previous.entries[0].id;

    let mut bytes = fs::read(&track).unwrap();
    let packed = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
    let total_samples = packed & ((1_u64 << 36) - 1);
    let sample_count_mask = (1_u64 << 36) - 1;
    let changed_packed = (packed & !sample_count_mask) | (total_samples + 1);
    bytes[18..26].copy_from_slice(&changed_packed.to_be_bytes());
    fs::write(&track, bytes).unwrap();
    let result = scan_library(&previous, std::slice::from_ref(&root));

    assert_eq!(result.media_errors.len(), 1);
    assert_eq!(result.catalog.entries.len(), 1);
    assert_eq!(result.catalog.entries[0].id, old_id);
    assert_eq!(result.catalog.entries[0].play_count, 3);
    assert!(result.catalog.entries[0].active);
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
    write_tagged_pcm_wav_with_album(path, "Live set");
}

fn write_tagged_pcm_wav_with_album(path: &std::path::Path, album: &str) {
    write_tagged_pcm_wav_with_sample(path, album, 0);
}

fn write_tagged_pcm_wav_with_sample(path: &std::path::Path, album: &str, sample: u8) {
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
    info_item(b"IPRD", album, &mut info);
    info_item(b"IGNR", "Ambient music", &mut info);
    info_item(b"ICRD", "2000-01-01", &mut info);
    chunk(b"LIST", &info, &mut wav_body);
    chunk(b"data", &vec![sample; 16_000], &mut wav_body);

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(wav_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&wav_body);
    fs::write(path, file).unwrap();
}

fn replace_bytes_once(path: &std::path::Path, from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let mut bytes = fs::read(path).unwrap();
    let start = bytes
        .windows(from.len())
        .position(|window| window == from)
        .unwrap_or_else(|| panic!("tag value not found in {}", path.display()));
    bytes[start..start + to.len()].copy_from_slice(to);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "ogg" | "opus"))
    {
        refresh_ogg_checksums(&mut bytes);
    }
    fs::write(path, bytes).unwrap();
}

fn refresh_ogg_checksums(bytes: &mut [u8]) {
    let mut offset = 0;
    while offset < bytes.len() {
        assert_eq!(&bytes[offset..offset + 4], b"OggS");
        let segment_count = bytes[offset + 26] as usize;
        let lacing_start = offset + 27;
        let body_start = lacing_start + segment_count;
        let body_size = bytes[lacing_start..body_start]
            .iter()
            .map(|size| usize::from(*size))
            .sum::<usize>();
        let page_end = body_start + body_size;
        bytes[offset + 22..offset + 26].fill(0);
        let mut crc = 0_u32;
        for &byte in &bytes[offset..page_end] {
            crc ^= u32::from(byte) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04c1_1db7
                } else {
                    crc << 1
                };
            }
        }
        bytes[offset + 22..offset + 26].copy_from_slice(&crc.to_le_bytes());
        offset = page_end;
    }
}

fn rewrite_ogg_serial(bytes: &mut [u8], serial: u32) {
    let mut offset = 0;
    while offset < bytes.len() {
        let segment_count = bytes[offset + 26] as usize;
        let lacing_start = offset + 27;
        let body_start = lacing_start + segment_count;
        let body_size = bytes[lacing_start..body_start]
            .iter()
            .map(|size| usize::from(*size))
            .sum::<usize>();
        let page_end = body_start + body_size;
        bytes[offset + 14..offset + 18].copy_from_slice(&serial.to_le_bytes());
        offset = page_end;
    }
    refresh_ogg_checksums(bytes);
}
