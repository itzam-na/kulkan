use std::path::PathBuf;
use std::sync::OnceLock;

use gst::prelude::*;
use gstreamer as gst;
use kulkan_ipc::{Message, PlaybackSnapshot, PlaybackStatus};

pub(super) struct PlaybackService {
    audio_sink: Option<String>,
    pipeline: Option<gst::Element>,
    snapshot: PlaybackSnapshot,
}

impl PlaybackService {
    pub(super) fn new(audio_sink: Option<String>) -> Self {
        Self {
            audio_sink,
            pipeline: None,
            snapshot: PlaybackSnapshot {
                track: None,
                status: PlaybackStatus::Stopped,
                position_ms: 0,
                duration_ms: None,
                error: None,
            },
        }
    }

    pub(super) fn handles(message: &Message) -> bool {
        matches!(
            message,
            Message::PlayTrack(_)
                | Message::PausePlayback
                | Message::ResumePlayback
                | Message::SeekPlayback(_)
                | Message::StopPlayback
                | Message::GetPlaybackState
        )
    }

    pub(super) fn handle(&mut self, message: Message) -> Message {
        let result = match message {
            Message::PlayTrack(path) => self.play(path),
            Message::PausePlayback => self.pause(),
            Message::ResumePlayback => self.resume(),
            Message::SeekPlayback(position_ms) => self.seek(position_ms),
            Message::StopPlayback => self.stop(),
            Message::GetPlaybackState => Ok(self.state()),
            _ => unreachable!("only playback messages are routed here"),
        };
        match result {
            Ok(snapshot) => Message::PlaybackState(snapshot),
            Err(error) => Message::PlaybackOperationFailed(error),
        }
    }

    fn play(&mut self, path: PathBuf) -> Result<PlaybackSnapshot, String> {
        self.stop_pipeline();
        ensure_gstreamer_initialized()?;
        if !path.is_file() {
            return self.fail(format!("Audio file is not available: {}", path.display()));
        }
        let uri = gst::glib::filename_to_uri(&path, None)
            .map_err(|error| format!("Could not open audio file: {error}"))?;
        let pipeline = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|error| format!("GStreamer playback is unavailable: {error}"))?;
        pipeline.set_property("uri", uri.as_str());
        if let Some(audio_sink) = &self.audio_sink {
            let sink = gst::ElementFactory::make(audio_sink)
                .build()
                .map_err(|error| {
                    format!("GStreamer audio sink {audio_sink} is unavailable: {error}")
                })?;
            if audio_sink == "fakesink" {
                sink.set_property("sync", true);
            }
            pipeline.set_property("audio-sink", sink);
        }
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| format!("GStreamer could not start playback: {error}"))?;
        self.pipeline = Some(pipeline);
        self.snapshot = PlaybackSnapshot {
            track: Some(path),
            status: PlaybackStatus::Playing,
            position_ms: 0,
            duration_ms: None,
            error: None,
        };
        Ok(self.state())
    }

    fn pause(&mut self) -> Result<PlaybackSnapshot, String> {
        let pipeline = self.active_pipeline()?;
        pipeline
            .set_state(gst::State::Paused)
            .map_err(|error| format!("GStreamer could not pause playback: {error}"))?;
        self.snapshot.status = PlaybackStatus::Paused;
        Ok(self.state())
    }

    fn resume(&mut self) -> Result<PlaybackSnapshot, String> {
        if self.snapshot.status != PlaybackStatus::Paused {
            return Err("Playback is not paused".to_owned());
        }
        self.active_pipeline()?
            .set_state(gst::State::Playing)
            .map_err(|error| format!("GStreamer could not resume playback: {error}"))?;
        self.snapshot.status = PlaybackStatus::Playing;
        Ok(self.state())
    }

    fn seek(&mut self, position_ms: u64) -> Result<PlaybackSnapshot, String> {
        if !matches!(
            self.snapshot.status,
            PlaybackStatus::Playing | PlaybackStatus::Paused
        ) {
            return Err("There is no active track to seek".to_owned());
        }
        let pipeline = self.active_pipeline()?;
        pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                gst::ClockTime::from_mseconds(position_ms),
            )
            .map_err(|_| "This audio stream could not seek to that position".to_owned())?;
        self.snapshot.position_ms = position_ms;
        Ok(self.state())
    }

    fn stop(&mut self) -> Result<PlaybackSnapshot, String> {
        self.stop_pipeline();
        self.snapshot = PlaybackSnapshot {
            track: None,
            status: PlaybackStatus::Stopped,
            position_ms: 0,
            duration_ms: None,
            error: None,
        };
        Ok(self.snapshot.clone())
    }

    fn state(&mut self) -> PlaybackSnapshot {
        if self.pipeline.is_none() {
            return self.snapshot.clone();
        }
        let mut finished = false;
        let snapshot = {
            let pipeline = self.pipeline.as_ref().expect("pipeline checked above");
            if let Some(bus) = pipeline.bus() {
                while let Some(message) = bus.pop() {
                    match message.view() {
                        gst::MessageView::Eos(..) => {
                            self.snapshot.status = PlaybackStatus::Ended;
                            finished = true;
                        }
                        gst::MessageView::Error(error) => {
                            self.snapshot.status = PlaybackStatus::Failed;
                            self.snapshot.error = Some(format!(
                                "GStreamer could not play this file: {}",
                                error.error()
                            ));
                            finished = true;
                        }
                        _ => {}
                    }
                }
            }
            if let Some(position) = pipeline.query_position::<gst::ClockTime>() {
                self.snapshot.position_ms = position.mseconds();
            }
            if let Some(duration) = pipeline.query_duration::<gst::ClockTime>() {
                self.snapshot.duration_ms = Some(duration.mseconds());
            }
            self.snapshot.clone()
        };
        if finished {
            self.stop_pipeline();
        }
        snapshot
    }

    fn active_pipeline(&self) -> Result<&gst::Element, String> {
        self.pipeline
            .as_ref()
            .ok_or_else(|| "No track is currently playing".to_owned())
    }

    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.snapshot.status = PlaybackStatus::Failed;
        self.snapshot.track = None;
        self.snapshot.error = Some(error.clone());
        Err(error)
    }

    fn stop_pipeline(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }
}

fn ensure_gstreamer_initialized() -> Result<(), String> {
    static GSTREAMER_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    GSTREAMER_INIT
        .get_or_init(|| {
            gst::init().map_err(|error| format!("could not initialize GStreamer: {error}"))
        })
        .clone()
}
