//! Sequential speech capture and local transcription.
//!
//! The crate emits `tracing` events for startup and captures.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod capture;
mod transcribe;

use earshot::{Control, Earshot};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// Configuration for one speech service.
#[derive(Debug, Clone)]
pub struct Config {
    /// The Whisper model file path.
    pub model: PathBuf,
    /// The language selection.
    pub language: Language,
    /// Use the GPU when this value is true.
    pub use_gpu: bool,
}

/// Language selection for transcription.
#[derive(Debug, Clone)]
pub enum Language {
    /// Detect the language for each capture.
    Auto,
    /// Use this language code.
    Code(String),
}

impl Language {
    fn as_str(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::Code(code) => code,
        }
    }
}

/// One capture result.
#[derive(Debug)]
pub enum Capture {
    /// Capture produced text.
    Speech(Transcript),
    /// Capture ended before speech started, or produced no text.
    NoSpeech,
    /// The caller stopped the service.
    Cancelled,
}

/// Transcript text and its endpoint status.
#[derive(Debug)]
pub struct Transcript {
    text: String,
    truncated: bool,
}

impl Transcript {
    fn new(text: String, truncated: bool) -> Option<Self> {
        (!text.is_empty()).then_some(Self { text, truncated })
    }

    /// Return the transcript text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return true when capture stopped at the duration limit.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Return the transcript text and drop the metadata.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }
}

/// A recoverable speech service failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The Whisper model did not load.
    #[error("cannot load Whisper model {path:?}: {source}")]
    Model {
        /// The model path.
        path: PathBuf,
        /// The Whisper failure.
        #[source]
        source: whisper_rs::WhisperError,
    },
    /// Earshot failed.
    #[error(transparent)]
    Earshot(#[from] earshot::Error),
    /// Whisper inference failed.
    #[error(transparent)]
    Whisper(#[from] whisper_rs::WhisperError),
    /// The configuration is invalid.
    #[error("invalid configuration: {0}")]
    Config(&'static str),
    /// The service stopped during startup.
    #[error("speech service was cancelled")]
    Cancelled,
    /// Earshot returned an outcome that this service does not handle.
    #[error("Earshot returned an unsupported outcome")]
    UnsupportedOutcome,
}

/// Route Whisper and GGML logs to the installed tracing subscriber.
///
/// Call this function once before `TetoCore::open`.
/// The subscriber controls output and filtering.
pub fn install_whisper_logging() {
    whisper_rs::install_logging_hooks();
}

fn validate_config(config: &Config) -> Result<(), Error> {
    if config.model.as_os_str().is_empty() {
        return Err(Error::Config("model path must not be empty"));
    }
    if let Language::Code(code) = &config.language
        && code.is_empty()
    {
        return Err(Error::Config("language must not be empty"));
    }
    Ok(())
}

pub(crate) fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// One process-lifetime speech service. Calls are sequential.
pub struct TetoCore {
    earshot: Earshot,
    control: Control,
    whisper: transcribe::Whisper,
    shutdown: Arc<AtomicBool>,
}

impl TetoCore {
    /// Load the Whisper model and open Earshot.
    ///
    /// The microphone stays closed until `next_utterance` runs.
    ///
    /// # Errors
    ///
    /// Returns a configuration, model, or Earshot startup error.
    /// Returns `Cancelled` when shutdown is set.
    pub fn open(config: Config, shutdown: Arc<AtomicBool>) -> Result<Self, Error> {
        let started = Instant::now();
        validate_config(&config)?;
        if shutdown.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }

        let model_bytes = std::fs::metadata(&config.model).map_or(0, |metadata| metadata.len());

        let whisper_started = Instant::now();
        let whisper = transcribe::Whisper::load(&config)?;
        let whisper_load_ms = elapsed_ms(whisper_started);

        if shutdown.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }

        let earshot_started = Instant::now();
        let earshot = Earshot::open()?;
        let earshot_open_ms = elapsed_ms(earshot_started);

        if shutdown.load(Ordering::Acquire) {
            earshot.close().map_err(Error::from)?;
            return Err(Error::Cancelled);
        }

        let control = earshot.control();
        tracing::info!(
            target: "teto_core::startup",
            pid = std::process::id() as u64,
            model = %config.model.display(),
            model_bytes = model_bytes,
            language = config.language.as_str(),
            gpu = config.use_gpu,
            whisper_cpp = whisper_rs::WHISPER_CPP_VERSION,
            core_version = env!("CARGO_PKG_VERSION"),
            debug = cfg!(debug_assertions),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            whisper_load_ms,
            earshot_open_ms,
            total_ms = elapsed_ms(started),
            "startup"
        );

        Ok(Self {
            earshot,
            control,
            whisper,
            shutdown,
        })
    }

    /// Activate capture, wait for one terminal outcome, and transcribe speech.
    ///
    /// # Errors
    ///
    /// Returns an activation, capture, or inference error.
    /// Call `close` after this method returns `Capture::Cancelled`.
    pub fn next_utterance(&mut self) -> Result<Capture, Error> {
        let started = Instant::now();
        if self.shutdown.load(Ordering::Acquire) {
            tracing::info!(
                target: "teto_core::capture",
                outcome = "cancelled",
                phase = "start",
                total_ms = elapsed_ms(started),
                "capture"
            );
            return Ok(Capture::Cancelled);
        }

        let Some(id) = self.activate_capture(started)? else {
            return Ok(Capture::Cancelled);
        };

        let settled = match capture::await_finished(&mut self.earshot, id, &self.shutdown) {
            Ok(settled) => settled,
            Err(error) => {
                tracing::error!(
                    target: "teto_core::capture",
                    id = id.get(),
                    phase = "capture",
                    error = %error,
                    total_ms = elapsed_ms(started),
                    "capture failed"
                );
                return Err(error);
            }
        };

        match settled {
            capture::Settled::Speech {
                audio,
                endpoint,
                timings,
            } => self.finish_speech(id, audio, endpoint, timings, started),
            capture::Settled::NoSpeech => {
                tracing::info!(
                    target: "teto_core::capture",
                    id = id.get(),
                    outcome = "no_speech",
                    endpoint = "none",
                    truncated = false,
                    total_ms = elapsed_ms(started),
                    "capture"
                );
                Ok(Capture::NoSpeech)
            }
            capture::Settled::Cancelled => {
                tracing::info!(
                    target: "teto_core::capture",
                    id = id.get(),
                    outcome = "cancelled",
                    endpoint = "none",
                    truncated = false,
                    total_ms = elapsed_ms(started),
                    "capture"
                );
                Ok(Capture::Cancelled)
            }
        }
    }

    fn activate_capture(&self, started: Instant) -> Result<Option<earshot::ActivationId>, Error> {
        match self.control.activate() {
            Ok(id) => Ok(Some(id)),
            Err(earshot::Error::Closed) if self.shutdown.load(Ordering::Acquire) => {
                tracing::info!(
                    target: "teto_core::capture",
                    outcome = "cancelled",
                    phase = "activate",
                    total_ms = elapsed_ms(started),
                    "capture"
                );
                Ok(None)
            }
            Err(error) => {
                tracing::error!(
                    target: "teto_core::capture",
                    phase = "activate",
                    error = %error,
                    total_ms = elapsed_ms(started),
                    "capture failed"
                );
                Err(error.into())
            }
        }
    }

    fn finish_speech(
        &mut self,
        id: earshot::ActivationId,
        audio: earshot::Audio,
        endpoint: capture::Endpoint,
        timings: earshot::Timings,
        started: Instant,
    ) -> Result<Capture, Error> {
        let endpoint_name = endpoint.as_str();
        let truncated = endpoint.truncated();
        let audio_ms = audio.len() as f64 / 16.0;

        let run = match self.whisper.run(audio.samples(), &self.shutdown) {
            Ok(run) => run,
            Err(error) => {
                tracing::error!(
                    target: "teto_core::capture",
                    id = id.get(),
                    endpoint = endpoint_name,
                    audio_ms,
                    phase = "transcribe",
                    error = %error,
                    total_ms = elapsed_ms(started),
                    "capture failed"
                );
                return Err(error);
            }
        };

        log_speech(id, endpoint, audio.len(), &timings, &run, started);

        match run.text {
            None => Ok(Capture::Cancelled),
            Some(text) => Ok(match Transcript::new(text, truncated) {
                Some(transcript) => Capture::Speech(transcript),
                None => Capture::NoSpeech,
            }),
        }
    }

    /// Set shutdown, close Earshot, and join its workers.
    ///
    /// Dropping the value also stops Earshot. Call this method to return the
    /// stop error and to set the shared shutdown flag.
    pub fn close(self) -> Result<(), Error> {
        self.shutdown.store(true, Ordering::Release);
        self.earshot.close().map_err(Error::from)
    }
}

fn log_speech(
    id: earshot::ActivationId,
    endpoint: capture::Endpoint,
    samples: usize,
    timings: &earshot::Timings,
    run: &transcribe::RunResult,
    started: Instant,
) {
    let audio_ms = samples as f64 / 16.0;
    let samples = samples as u64;
    let outcome = match &run.text {
        None => "cancelled",
        Some(text) if text.is_empty() => "no_speech",
        Some(_) => "speech",
    };
    let chars = run
        .text
        .as_ref()
        .map_or(0, |text| text.chars().count() as u64);

    tracing::info!(
        target: "teto_core::capture",
        id = id.get(),
        outcome,
        endpoint = endpoint.as_str(),
        truncated = endpoint.truncated(),
        audio_ms,
        samples,
        speech_onset_ms = timings.onset_sample.unwrap_or(0) as f64 / 16.0,
        speech_last_ms = timings.last_speech_sample.unwrap_or(0) as f64 / 16.0,
        activation_to_ready_ms = timings.activation_to_ready_ms,
        decision_after_ready_ms = timings.decision_after_ready_ms,
        capture_stop_ms = timings.capture_stop_ms,
        vad_frames = timings.vad_frames,
        vad_max_ms = timings.vad_max_ms,
        endpoint_calls = timings.endpoint_calls,
        endpoint_max_ms = timings.endpoint_max_ms,
        state_init_ms = run.state_init_ms,
        decode_ms = run.decode_ms,
        segments = run.segments,
        chars,
        total_ms = elapsed_ms(started),
        "capture"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_rejects_empty_text() {
        assert!(Transcript::new(String::new(), false).is_none());

        let transcript = Transcript::new("hello".into(), true).unwrap();
        assert_eq!(transcript.text(), "hello");
        assert!(transcript.is_truncated());
    }

    #[test]
    fn open_rejects_empty_model_path() {
        let config = Config {
            model: PathBuf::new(),
            language: Language::Code("en".into()),
            use_gpu: false,
        };
        let result = TetoCore::open(config, Arc::new(AtomicBool::new(false)));
        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[test]
    fn open_rejects_empty_language() {
        let config = Config {
            model: PathBuf::from("unused"),
            language: Language::Code(String::new()),
            use_gpu: false,
        };
        let result = TetoCore::open(config, Arc::new(AtomicBool::new(false)));
        assert!(matches!(result, Err(Error::Config(_))));
    }

    #[test]
    fn open_returns_cancelled_when_shutdown_is_set() {
        let config = Config {
            model: PathBuf::from("unused"),
            language: Language::Code("en".into()),
            use_gpu: false,
        };
        let shutdown = Arc::new(AtomicBool::new(true));
        let result = TetoCore::open(config, shutdown);
        assert!(matches!(result, Err(Error::Cancelled)));
    }
}

#[cfg(test)]
impl TetoCore {
    fn from_parts(
        earshot: Earshot,
        whisper: transcribe::Whisper,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let control = earshot.control();
        Self {
            earshot,
            control,
            whisper,
            shutdown,
        }
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use earshot::tooling::{InputSource, Policy, ReplayPacing};
    use std::path::Path;
    mod llm_replay;

    #[test]
    #[ignore = "loads Earshot engines and the Whisper model"]
    fn replay_speech_then_silence() {
        install_whisper_logging();

        let mut core = replay_core();

        let first = core.next_utterance().unwrap();
        assert!(matches!(
            first,
            Capture::Speech(ref transcript)
                if !transcript.text().is_empty() && !transcript.is_truncated()
        ));

        let second = core.next_utterance().unwrap();
        assert!(matches!(second, Capture::NoSpeech));

        core.close().unwrap();
    }
    fn replay_core() -> TetoCore {
        let model = std::env::var_os("TETO_WHISPER_MODEL").expect("set TETO_WHISPER_MODEL");
        let config = Config {
            model: model.into(),
            language: Language::Code("en".into()),
            use_gpu: false,
        };
        let whisper = transcribe::Whisper::load(&config).unwrap();

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../earshot/fixtures/speech-silence.wav");
        let mut reader = hound::WavReader::open(fixture).unwrap();
        let mut speech: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / 32768.0)
            .collect();
        speech.resize(speech.len() + 5 * 16_000, 0.0);
        let speech: Arc<[f32]> = Arc::from(speech);
        let silence: Arc<[f32]> = Arc::from(vec![0.0f32; 6 * 16_000]);
        let mut sources = vec![
            InputSource::samples(speech, ReplayPacing::Accelerated),
            InputSource::samples(silence, ReplayPacing::Accelerated),
        ]
        .into_iter();

        let earshot = earshot::tooling::start(
            move || sources.next().ok_or(earshot::Error::SourceEnded),
            Policy::default(),
            None,
        )
        .unwrap();

        let shutdown = Arc::new(AtomicBool::new(false));
        TetoCore::from_parts(earshot, whisper, shutdown)
    }
}
