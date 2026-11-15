use std::path::{Path, PathBuf};

use kulkan_ipc::{PlaybackSnapshot, PlaybackStatus};

pub(super) struct PlaybackAttempt {
    track: PathBuf,
    threshold_percent: u8,
    heard_segments: Vec<(u64, u64)>,
    last_position_ms: u64,
    qualified: bool,
    seeked_to_end: bool,
}

impl PlaybackAttempt {
    pub(super) fn new(track: PathBuf, threshold_percent: u8, position_ms: u64) -> Self {
        Self {
            track,
            threshold_percent,
            heard_segments: Vec::new(),
            last_position_ms: position_ms,
            qualified: false,
            seeked_to_end: false,
        }
    }

    pub(super) fn observe(&mut self, snapshot: &PlaybackSnapshot) -> bool {
        if snapshot.track.as_ref() != Some(&self.track) {
            return false;
        }

        if self.seeked_to_end
            && snapshot.status == PlaybackStatus::Playing
            && snapshot.position_ms > self.last_position_ms
        {
            self.seeked_to_end = false;
        }
        if matches!(
            snapshot.status,
            PlaybackStatus::Playing
                | PlaybackStatus::Paused
                | PlaybackStatus::Ended
                | PlaybackStatus::Failed
        ) {
            self.add_heard_segment(self.last_position_ms, snapshot.position_ms);
        }
        self.last_position_ms = snapshot.position_ms;

        !self.qualified && self.should_qualify(snapshot)
    }

    pub(super) fn track(&self) -> &Path {
        &self.track
    }

    pub(super) fn mark_qualified(&mut self) {
        self.qualified = true;
    }

    pub(super) fn seeked(&mut self, position_ms: u64, snapshot: &PlaybackSnapshot) {
        self.last_position_ms = snapshot.position_ms;
        self.seeked_to_end = snapshot
            .duration_ms
            .is_none_or(|duration| duration == 0 || position_ms >= duration);
    }

    fn add_heard_segment(&mut self, start_ms: u64, end_ms: u64) {
        if start_ms >= end_ms {
            return;
        }
        let mut start = start_ms;
        let mut end = end_ms;
        let mut merged = Vec::with_capacity(self.heard_segments.len() + 1);
        let mut inserted = false;
        for (segment_start, segment_end) in self.heard_segments.drain(..) {
            if segment_end < start {
                merged.push((segment_start, segment_end));
            } else if end < segment_start {
                if !inserted {
                    merged.push((start, end));
                    inserted = true;
                }
                merged.push((segment_start, segment_end));
            } else {
                start = start.min(segment_start);
                end = end.max(segment_end);
            }
        }
        if !inserted {
            merged.push((start, end));
        }
        self.heard_segments = merged;
    }

    fn should_qualify(&self, snapshot: &PlaybackSnapshot) -> bool {
        match snapshot.duration_ms.filter(|duration| *duration > 0) {
            Some(duration) => {
                u128::from(self.heard_milliseconds()) * 100
                    >= u128::from(duration) * u128::from(self.threshold_percent)
            }
            None => snapshot.status == PlaybackStatus::Ended && !self.seeked_to_end,
        }
    }

    fn heard_milliseconds(&self) -> u64 {
        self.heard_segments
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .fold(0, u64::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use super::PlaybackAttempt;
    use kulkan_ipc::{PlaybackSnapshot, PlaybackStatus};
    use std::path::PathBuf;

    fn snapshot(
        track: &str,
        status: PlaybackStatus,
        position_ms: u64,
        duration_ms: Option<u64>,
    ) -> PlaybackSnapshot {
        PlaybackSnapshot {
            track: Some(PathBuf::from(track)),
            status,
            position_ms,
            duration_ms,
            error: None,
        }
    }

    #[test]
    fn attempt_qualifies_after_hearing_the_configured_share() {
        let mut attempt = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);
        let snapshot = snapshot("track.wav", PlaybackStatus::Playing, 3_750, Some(5_000));

        assert!(attempt.observe(&snapshot));
        assert!(attempt.observe(&snapshot));
        attempt.mark_qualified();
        assert!(!attempt.observe(&snapshot));
    }

    #[test]
    fn replayed_content_contributes_to_coverage_only_once() {
        let mut attempt = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);

        assert!(!attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Playing,
            2_000,
            Some(5_000),
        )));
        attempt.seeked(
            1_000,
            &snapshot("track.wav", PlaybackStatus::Playing, 1_000, Some(5_000)),
        );
        assert!(!attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Playing,
            3_000,
            Some(5_000),
        )));
        assert!(attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Playing,
            3_750,
            Some(5_000),
        )));
    }

    #[test]
    fn unknown_duration_requires_natural_completion_after_a_seek_to_end() {
        let mut natural = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);
        assert!(natural.observe(&snapshot("track.wav", PlaybackStatus::Ended, 0, None,)));

        let mut seeked = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);
        seeked.seeked(
            5_000,
            &snapshot("track.wav", PlaybackStatus::Playing, 0, None),
        );
        assert!(!seeked.observe(&snapshot("track.wav", PlaybackStatus::Ended, 0, None,)));

        assert!(!seeked.observe(&snapshot("track.wav", PlaybackStatus::Playing, 500, None,)));
        assert!(seeked.observe(&snapshot("track.wav", PlaybackStatus::Ended, 500, None,)));
    }

    #[test]
    fn a_failed_snapshot_qualifies_if_enough_distinct_content_was_heard() {
        let mut attempt = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);

        assert!(attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Failed,
            3_750,
            Some(5_000),
        )));
    }

    #[test]
    fn another_track_and_stopped_position_do_not_add_heard_coverage() {
        let mut attempt = PlaybackAttempt::new(PathBuf::from("track.wav"), 75, 0);

        assert!(!attempt.observe(&snapshot(
            "other.wav",
            PlaybackStatus::Playing,
            3_750,
            Some(5_000),
        )));
        assert!(!attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Stopped,
            3_750,
            Some(5_000),
        )));
        assert!(!attempt.observe(&snapshot(
            "track.wav",
            PlaybackStatus::Playing,
            3_750,
            Some(5_000),
        )));
    }
}
