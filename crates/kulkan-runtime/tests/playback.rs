use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kulkan_ipc::PlaybackStatus;
use kulkan_runtime::{RepeatMode, ServerProcess};

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
fn a_threshold_change_does_not_change_an_attempt_that_is_already_playing() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();

    server.play_track(&track).unwrap();
    server.set_played_threshold(100).unwrap();
    wait_for_position(&server, 200);

    assert_eq!(play_count(&server, &track), 1);
    assert_eq!(server.played_threshold().unwrap(), 100);
    server.shutdown().unwrap();
}

#[test]
fn seeking_adds_no_coverage_and_replayed_sections_count_only_once() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.play_track(&track).unwrap();

    wait_for_position(&server, 1_000);
    server.seek_playback(2_900).unwrap();
    wait_for_position(&server, 3_400);
    server.seek_playback(500).unwrap();
    wait_for_position(&server, 2_400);
    assert_eq!(play_count(&server, &track), 0);

    wait_for_position(&server, 2_700);
    assert_eq!(play_count(&server, &track), 1);
    server.shutdown().unwrap();
}

#[test]
fn early_stop_adds_no_play_and_a_qualified_play_survives_stop() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();

    server.play_track(&track).unwrap();
    wait_for_position(&server, 600);
    server.pause_playback().unwrap();
    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        server.playback_state().unwrap().status,
        PlaybackStatus::Paused
    );
    server.resume_playback().unwrap();
    wait_for_position(&server, 1_000);
    server.stop_playback().unwrap();
    assert_eq!(play_count(&server, &track), 0);

    server.set_played_threshold(1).unwrap();
    server.play_track(&track).unwrap();
    wait_for_position(&server, 200);
    assert_eq!(play_count(&server, &track), 1);
    server.stop_playback().unwrap();
    assert_eq!(play_count(&server, &track), 1);
    server.shutdown().unwrap();
}

#[test]
fn listened_coverage_is_preserved_across_a_pause() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.play_track(&track).unwrap();

    wait_for_position(&server, 1_700);
    server.pause_playback().unwrap();
    let paused_position = server.playback_state().unwrap().position_ms;
    thread::sleep(Duration::from_millis(100));
    assert!(server.playback_state().unwrap().position_ms <= paused_position + 5);
    assert_eq!(play_count(&server, &track), 0);

    server.resume_playback().unwrap();
    wait_for_position(&server, 3_100);

    assert_eq!(play_count(&server, &track), 1);
    server.shutdown().unwrap();
}

#[test]
fn skipping_before_threshold_adds_nothing_and_skipping_after_it_keeps_the_count() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let tracks = ["one.wav", "two.wav", "three.wav"].map(|name| {
        let track = root.join(name);
        write_pcm_wav(&track, 32_000);
        track
    });
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    let queue = server.append_to_queue(&tracks).unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();
    wait_for_position(&server, 600);

    let after_early_skip = server.skip_queue_entry().unwrap();
    assert_eq!(after_early_skip.current_entry_id, Some(queue.entries[1].id));
    assert_eq!(play_count(&server, &tracks[0]), 0);

    server.set_played_threshold(1).unwrap();
    server.play_queue_entry(queue.entries[2].id).unwrap();
    wait_for_position(&server, 200);
    assert_eq!(play_count(&server, &tracks[2]), 1);
    server.skip_queue_entry().unwrap();

    assert_eq!(play_count(&server, &tracks[2]), 1);
    server.shutdown().unwrap();
}

#[test]
fn each_repeat_one_pass_can_qualify_once() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();
    let queue = server
        .append_to_queue(std::slice::from_ref(&track))
        .unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::One)
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    wait_for_count(&server, &track, 1);
    wait_for_count(&server, &track, 2);

    assert_eq!(play_count(&server, &track), 2);
    server.shutdown().unwrap();
}

#[test]
fn seeking_to_the_end_does_not_qualify_a_track_with_known_duration() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.play_track(&track).unwrap();
    wait_until(&server, |state| state.duration_ms.is_some());

    server.seek_playback(3_990).unwrap();
    wait_for_status(&server, PlaybackStatus::Ended);

    assert_eq!(play_count(&server, &track), 0);
    server.shutdown().unwrap();
}

#[test]
fn history_can_be_cleared_for_one_or_all_inactive_entries_without_removing_them() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let tracks = ["one.wav", "two.wav"].map(|name| {
        let track = root.join(name);
        write_pcm_wav(&track, 32_000);
        track
    });
    let mut server = start_headless_server(&directory);
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();

    for track in &tracks {
        server.play_track(track).unwrap();
        wait_for_count(&server, track, 1);
        server.stop_playback().unwrap();
    }
    server.remove_library_root(&root).unwrap();
    assert_eq!(server.scan_library().unwrap().inactive_entries, 2);

    server.clear_entry_play_history(&tracks[0]).unwrap();
    let after_one_clear = server.catalog_page(0, 32).unwrap().entries;
    assert_eq!(after_one_clear.len(), 2);
    assert!(after_one_clear.iter().all(|entry| !entry.active));
    assert_eq!(play_count(&server, &tracks[0]), 0);
    assert_eq!(play_count(&server, &tracks[1]), 1);

    server.clear_all_play_history().unwrap();
    let after_all_clear = server.catalog_page(0, 32).unwrap().entries;
    assert_eq!(after_all_clear.len(), 2);
    assert!(after_all_clear.iter().all(|entry| !entry.active));
    assert!(after_all_clear.iter().all(|entry| entry.play_count == 0));
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

#[test]
fn queue_accepts_duplicate_occurrences_without_starting_playback() {
    let directory = TestDirectory::new();
    let first_track = directory.0.join("first.wav");
    write_pcm_wav(&first_track, 80_000);
    let mut server = start_headless_server(&directory);

    let queue = server
        .append_to_queue(&[first_track.clone(), first_track.clone()])
        .unwrap();

    assert_eq!(queue.entries.len(), 2);
    assert_ne!(queue.entries[0].id, queue.entries[1].id);
    assert_eq!(queue.entries[0].track, first_track);
    assert_eq!(queue.entries[1].track, first_track);
    assert_eq!(queue.current_entry_id, None);
    assert_eq!(queue.playback.status, PlaybackStatus::Stopped);
    server.shutdown().unwrap();
}

#[test]
fn queue_playback_starts_the_requested_occurrence() {
    let directory = TestDirectory::new();
    let first_track = directory.0.join("first.wav");
    let second_track = directory.0.join("second.wav");
    write_pcm_wav(&first_track, 80_000);
    write_pcm_wav(&second_track, 80_000);
    let mut server = start_headless_server(&directory);
    let queue = server
        .append_to_queue(&[first_track.clone(), second_track.clone()])
        .unwrap();

    let started = server.play_queue_entry(queue.entries[1].id).unwrap();

    assert_eq!(started.current_entry_id, Some(queue.entries[1].id));
    assert_eq!(started.playback.track, Some(second_track));
    assert_eq!(started.playback.status, PlaybackStatus::Playing);
    assert!(!started.entries[1].failed);
    server.shutdown().unwrap();
}

#[test]
fn queue_advances_to_the_next_occurrence_after_a_track_ends() {
    let directory = TestDirectory::new();
    let first_track = directory.0.join("first.wav");
    let second_track = directory.0.join("second.wav");
    write_pcm_wav(&first_track, 4_000);
    write_pcm_wav(&second_track, 12_000);
    let mut server = start_headless_server(&directory);
    let queue = server
        .append_to_queue(&[first_track, second_track.clone()])
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    let advanced = wait_for_queue(&server, |state| {
        state.current_entry_id == Some(queue.entries[1].id)
    });

    assert_eq!(advanced.playback.track, Some(second_track));
    assert_eq!(advanced.playback.status, PlaybackStatus::Playing);
    let ended = wait_for_queue(&server, |state| {
        state.playback.status == PlaybackStatus::Ended
    });
    assert_eq!(ended.entries.len(), 2);
    assert_eq!(ended.current_entry_id, Some(queue.entries[1].id));
    server.shutdown().unwrap();
}

#[test]
fn queue_repeat_one_restarts_the_current_occurrence() {
    let directory = TestDirectory::new();
    let track = directory.0.join("repeat.wav");
    write_pcm_wav(&track, 4_000);
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&[track]).unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::One)
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();
    wait_for_queue(&server, |state| state.playback.position_ms >= 250);

    let repeated = wait_for_queue(&server, |state| {
        state.current_entry_id == Some(queue.entries[0].id)
            && state.playback.status == PlaybackStatus::Playing
            && state.playback.position_ms < 100
    });

    assert_eq!(repeated.repeat_mode, kulkan_runtime::RepeatMode::One);
    server.shutdown().unwrap();
}

#[test]
fn queue_marks_a_failed_entry_and_continues_with_the_next_playable_entry() {
    let directory = TestDirectory::new();
    let missing_track = directory.0.join("missing.wav");
    let next_track = directory.0.join("next.wav");
    write_pcm_wav(&next_track, 80_000);
    let mut server = start_headless_server(&directory);
    let queue = server
        .append_to_queue(&[missing_track, next_track.clone()])
        .unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::One)
        .unwrap();

    let state = server.play_queue_entry(queue.entries[0].id).unwrap();

    assert!(state.entries[0].failed);
    assert_eq!(state.current_entry_id, Some(queue.entries[1].id));
    assert_eq!(state.playback.track, Some(next_track));
    assert_eq!(state.playback.status, PlaybackStatus::Playing);
    assert_eq!(state.repeat_mode, kulkan_runtime::RepeatMode::Off);
    server.shutdown().unwrap();
}

#[test]
fn removing_the_current_entry_advances_to_the_next_occurrence() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&tracks).unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    let after_removal = server.remove_queue_entry(queue.entries[0].id).unwrap();

    assert_eq!(after_removal.entries.len(), 1);
    assert_eq!(after_removal.current_entry_id, Some(queue.entries[1].id));
    assert_eq!(after_removal.playback.track, Some(tracks[1].clone()));
    assert_eq!(after_removal.playback.status, PlaybackStatus::Playing);
    server.shutdown().unwrap();
}

#[test]
fn skipping_advances_even_when_repeat_one_is_enabled() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&tracks).unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::One)
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    let skipped = server.skip_queue_entry().unwrap();

    assert_eq!(skipped.current_entry_id, Some(queue.entries[1].id));
    assert_eq!(skipped.playback.track, Some(tracks[1].clone()));
    assert_eq!(skipped.repeat_mode, kulkan_runtime::RepeatMode::One);
    server.shutdown().unwrap();
}

#[test]
fn repeat_all_starts_another_queue_cycle_without_dropping_entries() {
    let directory = TestDirectory::new();
    let track = directory.0.join("repeat-all.wav");
    write_pcm_wav(&track, 4_000);
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&[track]).unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::All)
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();
    wait_for_queue(&server, |state| state.playback.position_ms >= 250);

    let next_cycle = wait_for_queue(&server, |state| {
        state.current_entry_id == Some(queue.entries[0].id)
            && state.playback.status == PlaybackStatus::Playing
            && state.playback.position_ms < 100
    });

    assert_eq!(next_cycle.entries.len(), 1);
    assert_eq!(next_cycle.repeat_mode, kulkan_runtime::RepeatMode::All);
    server.shutdown().unwrap();
}

#[test]
fn shuffled_repeat_all_plays_each_occurrence_once_before_the_next_cycle() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav", "three.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 4_000);
        track
    });
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&tracks).unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::All)
        .unwrap();
    let shuffled = server.set_shuffle(true).unwrap();
    let first_id = shuffled.entries[0].id;
    let mut state = server.play_queue_entry(first_id).unwrap();
    let mut previous_id = first_id;
    let mut previous_position = state.playback.position_ms;
    let mut played_once = vec![first_id];
    let deadline = Instant::now() + Duration::from_secs(8);

    loop {
        state = server.queue_state().unwrap();
        let current_id = state.current_entry_id.unwrap();
        if current_id != previous_id {
            if played_once.contains(&current_id) {
                break;
            }
            played_once.push(current_id);
            previous_id = current_id;
        } else if previous_position >= 250 && state.playback.position_ms < 100 {
            break;
        }
        previous_position = state.playback.position_ms;
        assert!(
            Instant::now() < deadline,
            "shuffle cycle did not complete: {state:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(played_once.len(), queue.entries.len());
    assert_eq!(state.entries.len(), queue.entries.len());
    assert_eq!(state.repeat_mode, kulkan_runtime::RepeatMode::All);
    server.shutdown().unwrap();
}

#[test]
fn failed_queue_entries_stay_skipped_on_later_repeat_all_cycles() {
    let directory = TestDirectory::new();
    let missing_track = directory.0.join("missing.wav");
    let playable_track = directory.0.join("playable.wav");
    write_pcm_wav(&playable_track, 4_000);
    let mut server = start_headless_server(&directory);
    let queue = server
        .append_to_queue(&[missing_track, playable_track.clone()])
        .unwrap();
    server
        .set_repeat_mode(kulkan_runtime::RepeatMode::All)
        .unwrap();

    let first_cycle = server.play_queue_entry(queue.entries[0].id).unwrap();
    assert!(first_cycle.entries[0].failed);
    assert_eq!(first_cycle.current_entry_id, Some(queue.entries[1].id));
    wait_for_queue(&server, |state| state.playback.position_ms >= 250);
    let second_cycle = wait_for_queue(&server, |state| {
        state.current_entry_id == Some(queue.entries[1].id)
            && state.playback.status == PlaybackStatus::Playing
            && state.playback.position_ms < 100
    });

    assert!(second_cycle.entries[0].failed);
    assert_eq!(second_cycle.playback.track, Some(playable_track));
    server.shutdown().unwrap();
}

#[test]
fn a_successful_final_track_ends_normally_after_an_earlier_failure() {
    let directory = TestDirectory::new();
    let missing_track = directory.0.join("missing.wav");
    let final_track = directory.0.join("final.wav");
    write_pcm_wav(&final_track, 4_000);
    let mut server = start_headless_server(&directory);
    let queue = server
        .append_to_queue(&[missing_track, final_track])
        .unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    let ended = wait_for_queue(&server, |state| {
        state.playback.status == PlaybackStatus::Ended
    });

    assert!(ended.entries[0].failed);
    assert_eq!(ended.current_entry_id, Some(queue.entries[1].id));
    assert!(ended.playback.error.is_none());
    assert_eq!(ended.entries.len(), 2);
    server.shutdown().unwrap();
}

#[test]
fn shuffle_off_preserves_manual_queue_reordering_and_clear_stops_playback() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav", "three.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let mut server = start_headless_server(&directory);
    server.append_to_queue(&tracks).unwrap();
    let shuffled = server.set_shuffle(true).unwrap();
    let moved_id = shuffled.entries[2].id;
    let reordered = server.reorder_queue_entry(moved_id, 0).unwrap();
    let order_while_shuffled = reordered
        .entries
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();

    let unshuffled = server.set_shuffle(false).unwrap();

    assert_eq!(
        unshuffled
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        order_while_shuffled
    );
    assert_eq!(unshuffled.entries[0].id, moved_id);
    server.play_queue_entry(moved_id).unwrap();
    let cleared = server.clear_queue().unwrap();
    assert!(cleared.entries.is_empty());
    assert_eq!(cleared.current_entry_id, None);
    assert_eq!(cleared.playback.status, PlaybackStatus::Stopped);
    server.shutdown().unwrap();
}

#[test]
fn shuffling_and_reordering_upcoming_entries_do_not_interrupt_the_current_track() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav", "three.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&tracks).unwrap();
    let started = server.play_queue_entry(queue.entries[0].id).unwrap();

    let shuffled = server.set_shuffle(true).unwrap();
    let moved_entry = shuffled.entries[2].id;
    let reordered = server.reorder_queue_entry(moved_entry, 1).unwrap();

    assert_eq!(reordered.current_entry_id, Some(queue.entries[0].id));
    assert_eq!(reordered.entries[1].id, moved_entry);
    assert_eq!(reordered.playback.track, started.playback.track);
    assert_eq!(reordered.playback.status, PlaybackStatus::Playing);
    server.shutdown().unwrap();
}

#[test]
fn previous_goes_back_before_two_seconds_and_restarts_the_current_track_afterward() {
    let directory = TestDirectory::new();
    let tracks = ["one.wav", "two.wav"].map(|name| {
        let track = directory.0.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let mut server = start_headless_server(&directory);
    let queue = server.append_to_queue(&tracks).unwrap();
    let previous_id = queue.entries[0].id;
    let current_id = queue.entries[1].id;

    server.play_queue_entry(current_id).unwrap();
    let returned_to_previous = server.previous_queue_entry().unwrap();
    assert_eq!(returned_to_previous.current_entry_id, Some(previous_id));
    assert_eq!(returned_to_previous.playback.track, Some(tracks[0].clone()));
    let previous_at_queue_start = server.previous_queue_entry().unwrap();
    assert_eq!(previous_at_queue_start.current_entry_id, Some(previous_id));
    assert!(previous_at_queue_start.playback.position_ms < 2_000);

    server.play_queue_entry(current_id).unwrap();
    wait_for_position(&server, 2_100);
    let restarted_current = server.previous_queue_entry().unwrap();

    assert_eq!(restarted_current.current_entry_id, Some(current_id));
    assert_eq!(restarted_current.playback.track, Some(tracks[1].clone()));
    assert!(restarted_current.playback.position_ms < 2_000);
    server.shutdown().unwrap();
}

#[test]
fn qualifying_play_counts_and_threshold_survive_server_relaunch() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let socket = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut first_server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    first_server.add_library_root(&root).unwrap();
    first_server.scan_library().unwrap();
    first_server.set_played_threshold(1).unwrap();
    first_server.play_track(&track).unwrap();
    wait_for_position(&first_server, 200);
    assert_eq!(play_count(&first_server, &track), 1);
    first_server.stop_playback().unwrap();
    first_server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();

    assert_eq!(restored_server.played_threshold().unwrap(), 1);
    assert_eq!(play_count(&restored_server, &track), 1);
    restored_server.shutdown().unwrap();
}

#[test]
fn queue_order_settings_and_current_entry_restore_from_the_beginning() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let tracks = ["one.wav", "two.wav", "three.wav"].map(|name| {
        let track = root.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let socket = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut first_server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    first_server.add_library_root(&root).unwrap();
    first_server.scan_library().unwrap();
    let appended = first_server.append_to_queue(&tracks).unwrap();
    let shuffled = first_server.set_shuffle(true).unwrap();
    let moved_entry_id = shuffled.entries[2].id;
    let reordered = first_server.reorder_queue_entry(moved_entry_id, 1).unwrap();
    let repeated = first_server.set_repeat_mode(RepeatMode::All).unwrap();
    let current_entry_id = repeated.entries[1].id;
    let current_track = repeated.entries[1].track.clone();
    first_server.play_queue_entry(current_entry_id).unwrap();
    wait_for_position(&first_server, 1_000);
    let before_restart = first_server.queue_state().unwrap();
    assert!(before_restart.playback.position_ms >= 1_000);
    first_server.shutdown().unwrap();

    let mut restored_server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    let restored = restored_server.queue_state().unwrap();

    assert_eq!(restored.entries, before_restart.entries);
    assert_eq!(restored.current_entry_id, Some(current_entry_id));
    assert_eq!(restored.repeat_mode, RepeatMode::All);
    assert!(restored.shuffle);
    assert_eq!(restored.playback.track, Some(current_track));
    assert_eq!(restored.playback.status, PlaybackStatus::Playing);
    assert!(restored.playback.position_ms < 1_000);
    let appended_after_restore = restored_server.append_to_queue(&tracks[0..1]).unwrap();
    let added_id = appended_after_restore.entries.last().unwrap().id;
    assert!(appended.entries.iter().all(|entry| entry.id != added_id));
    assert_eq!(reordered.entries[1].id, moved_entry_id);
    restored_server.shutdown().unwrap();
}

#[test]
fn app_launch_restores_the_current_track_paused_from_the_beginning() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("ten-seconds.wav");
    write_pcm_wav(&track, 80_000);
    let socket = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut first_server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    first_server.add_library_root(&root).unwrap();
    first_server.scan_library().unwrap();
    let queue = first_server
        .append_to_queue(std::slice::from_ref(&track))
        .unwrap();
    first_server.play_queue_entry(queue.entries[0].id).unwrap();
    wait_for_position(&first_server, 500);
    first_server.shutdown().unwrap();

    let mut launched_server = ServerProcess::start_with_audio_sink_and_paused_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        Some("fakesink"),
    )
    .unwrap();
    let restored = launched_server.queue_state().unwrap();

    assert_eq!(restored.current_entry_id, Some(queue.entries[0].id));
    assert_eq!(restored.playback.track, Some(track));
    assert_eq!(restored.playback.status, PlaybackStatus::Paused);
    assert!(restored.playback.position_ms < 100);
    launched_server.shutdown().unwrap();
}

#[test]
fn exported_state_restores_library_history_queue_settings_and_current_track() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let tracks = ["one.wav", "two.wav"].map(|name| {
        let track = root.join(name);
        write_pcm_wav(&track, 80_000);
        track
    });
    let original_media = tracks
        .iter()
        .map(|track| fs::read(track).unwrap())
        .collect::<Vec<_>>();
    let socket = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    fs::create_dir(directory.0.join("backup")).unwrap();
    let export_path = directory.0.join("backup/kulkan-state.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();
    server.play_track(&tracks[0]).unwrap();
    wait_for_count(&server, &tracks[0], 1);
    server.stop_playback().unwrap();
    server.set_played_threshold(100).unwrap();
    let queue = server.append_to_queue(&tracks).unwrap();
    server.set_shuffle(true).unwrap();
    server.set_repeat_mode(RepeatMode::All).unwrap();
    let current_entry_id = queue.entries[1].id;
    let selected = server.play_queue_entry(current_entry_id).unwrap();
    let current_track = selected
        .entries
        .iter()
        .find(|entry| entry.id == current_entry_id)
        .unwrap()
        .track
        .clone();
    wait_for_position(&server, 1_000);
    let original_queue = server.queue_state().unwrap();

    server.export_app_state(&export_path).unwrap();
    let export_bytes = fs::read(&export_path).unwrap();
    assert!(!export_bytes.contains(&0));
    server.clear_queue().unwrap();
    server.clear_all_play_history().unwrap();
    server.remove_library_root(&root).unwrap();
    server.scan_library().unwrap();

    server.import_app_state(&export_path).unwrap();
    let restored_library = server.library_state().unwrap();
    let restored_entries = server.catalog_page(0, 32).unwrap().entries;
    let restored_queue = server.queue_state().unwrap();

    assert_eq!(restored_library.selected_roots, vec![root.clone()]);
    assert_eq!(restored_entries.len(), 2);
    assert!(restored_entries.iter().all(|entry| entry.active));
    assert_eq!(play_count(&server, &tracks[0]), 1);
    assert_eq!(server.played_threshold().unwrap(), 100);
    assert_eq!(restored_queue.entries, original_queue.entries);
    assert_eq!(restored_queue.repeat_mode, RepeatMode::All);
    assert!(restored_queue.shuffle);
    assert_eq!(restored_queue.playback.track, Some(current_track));
    assert!(restored_queue.playback.position_ms < 1_000);
    assert_eq!(
        tracks
            .iter()
            .map(|track| fs::read(track).unwrap())
            .collect::<Vec<_>>(),
        original_media
    );
    server.shutdown().unwrap();
}

#[test]
fn resetting_app_state_clears_kulkan_data_without_changing_media_files() {
    let directory = TestDirectory::new();
    let root = directory.0.join("music");
    fs::create_dir(&root).unwrap();
    let track = root.join("four-seconds.wav");
    write_pcm_wav(&track, 32_000);
    let original_media = fs::read(&track).unwrap();
    let socket = directory.0.join("server.sock");
    let state_path = directory.0.join("state/library.json");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kulkan-server"));
    let mut server = ServerProcess::start_with_audio_sink_and_library_state(
        &executable,
        &socket,
        &state_path,
        Duration::from_secs(5),
        "fakesink",
    )
    .unwrap();
    server.add_library_root(&root).unwrap();
    server.scan_library().unwrap();
    server.set_played_threshold(1).unwrap();
    let queue = server
        .append_to_queue(std::slice::from_ref(&track))
        .unwrap();
    server.set_repeat_mode(RepeatMode::One).unwrap();
    server.play_queue_entry(queue.entries[0].id).unwrap();

    server.reset_app_state().unwrap();

    assert!(server.library_state().unwrap().selected_roots.is_empty());
    assert_eq!(server.library_state().unwrap().catalog_entries, 0);
    assert_eq!(server.played_threshold().unwrap(), 75);
    let queue = server.queue_state().unwrap();
    assert!(queue.entries.is_empty());
    assert_eq!(queue.current_entry_id, None);
    assert_eq!(queue.repeat_mode, RepeatMode::Off);
    assert!(!queue.shuffle);
    assert_eq!(queue.playback.status, PlaybackStatus::Stopped);
    assert_eq!(fs::read(&track).unwrap(), original_media);
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

fn play_count(server: &ServerProcess, track: &std::path::Path) -> u64 {
    server
        .catalog_page(0, 32)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.path == track)
        .expect("test track remains in the Library")
        .play_count
}

fn wait_for_count(server: &ServerProcess, track: &std::path::Path, expected: u64) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let _ = server.queue_state().unwrap();
        if play_count(server, track) >= expected {
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

fn wait_for_queue(
    server: &ServerProcess,
    predicate: impl Fn(&kulkan_ipc::QueueSnapshot) -> bool,
) -> kulkan_ipc::QueueSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = server.queue_state().unwrap();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "queue did not reach the expected state: {state:?}"
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
