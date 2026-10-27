//! SPACE activation and transcript output.
//!
//! `TETO_LOG` sets the tracing filter.
//! The default is `warn,teto_core=info,teto_cli=info`.
//! `TETO_LOG=off` disables logs.
//! `TETO_LOG=trace` shows Whisper token and kernel logs.
//! `TETO_LOG_FORMAT=json` selects JSON lines.
//! `TETO_WHISPER_MODEL` is required.
//! `TETO_WHISPER_LANGUAGE` defaults to `en`.
//! `TETO_WHISPER_GPU` defaults to true on macOS.

#![forbid(unsafe_code)]

mod journal_setup;
mod llm_client;
mod llm_demo;
mod terminal;

use std::{
    env,
    ffi::OsString,
    fmt,
    io::IsTerminal,
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use teto_core::{Capture, Config, Error, Language, TetoCore};
use tracing_subscriber::EnvFilter;

const DEFAULT_FILTER: &str = "warn,teto_core=info,teto_cli=info";

enum LoopError {
    Core(Error),
    Terminal(std::io::Error),
}

impl From<Error> for LoopError {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

impl From<std::io::Error> for LoopError {
    fn from(error: std::io::Error) -> Self {
        Self::Terminal(error)
    }
}

impl fmt::Display for LoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(formatter),
            Self::Terminal(error) => write!(formatter, "Cannot read terminal input: {error}"),
        }
    }
}

fn init_logging() {
    let filter =
        EnvFilter::try_from_env("TETO_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let ansi = std::io::stderr().is_terminal();
    let json = env::var("TETO_LOG_FORMAT").ok().as_deref() == Some("json");

    if json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(ansi)
            .init();
    }
}

fn main() -> ExitCode {
    init_logging();
    teto_core::install_whisper_logging();

    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_shutdown = Arc::clone(&shutdown);
    if let Err(error) = ctrlc::set_handler(move || {
        signal_shutdown.store(true, Ordering::Release);
    }) {
        tracing::error!(
            target: "teto_cli",
            error = %error,
            "cannot install Ctrl-C handler"
        );
        return ExitCode::FAILURE;
    }
    run(&shutdown)
}

fn run(shutdown: &Arc<AtomicBool>) -> ExitCode {
    let mode = env::var("TETO_MODE").unwrap_or_else(|_| "transcribe".into());
    let command = match parse_command(env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if command.is_some() || mode != "transcribe" {
        let result = match command.as_deref() {
            Some("journal-init") => journal_setup::initialize(),
            Some("journal-delete") => journal_setup::delete(),
            Some("qualify") => llm_demo::qualify(shutdown),
            Some("qualify-cold") => llm_demo::qualify_cold(shutdown),
            Some("qualify-lifecycle") => llm_demo::qualify_lifecycle(shutdown),
            Some(_) => Err(teto_llm_protocol::failure(
                teto_llm_protocol::ErrorKind::Config,
            )),
            None => llm_demo::run(&mode, shutdown),
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error.kind() == teto_llm_protocol::ErrorKind::Cancelled => {
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    run_transcribe(shutdown)
}

fn parse_command(
    mut arguments: impl Iterator<Item = String>,
) -> teto_llm_protocol::Result<Option<String>> {
    let command = arguments.next();
    if arguments.next().is_some() {
        return Err(teto_llm_protocol::failure(
            teto_llm_protocol::ErrorKind::Config,
        ));
    }
    Ok(command)
}

fn run_transcribe(shutdown: &Arc<AtomicBool>) -> ExitCode {
    let config = match config_from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(
                target: "teto_cli::startup",
                error = %error,
                "startup failed"
            );
            return ExitCode::FAILURE;
        }
    };

    let mut core = match TetoCore::open(config, Arc::clone(shutdown)) {
        Ok(core) => core,
        Err(error) => {
            if shutdown.load(Ordering::Acquire) || matches!(&error, Error::Cancelled) {
                return ExitCode::SUCCESS;
            }
            tracing::error!(
                target: "teto_cli::startup",
                error = %error,
                "startup failed"
            );
            return ExitCode::FAILURE;
        }
    };

    let result = capture_loop(&mut core, shutdown);
    let interrupted = shutdown.load(Ordering::Acquire);
    if !interrupted && let Err(LoopError::Terminal(error)) = &result {
        tracing::error!(
            target: "teto_cli::terminal",
            error = %error,
            "terminal failed"
        );
    }

    let close_started = Instant::now();
    let cleanup = core.close();
    let close_ms = elapsed_ms(close_started);
    match &cleanup {
        Ok(()) => tracing::info!(
            target: "teto_cli::shutdown",
            interrupted,
            elapsed_ms = close_ms,
            "shutdown"
        ),
        Err(error) => tracing::error!(
            target: "teto_cli::shutdown",
            interrupted,
            elapsed_ms = close_ms,
            error = %error,
            "shutdown failed"
        ),
    }

    if interrupted || (result.is_ok() && cleanup.is_ok()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn capture_loop(core: &mut TetoCore, shutdown: &AtomicBool) -> Result<(), LoopError> {
    while !shutdown.load(Ordering::Acquire) {
        eprintln!("Press SPACE to activate, Ctrl-C to quit.");
        if !terminal::wait_for_space(shutdown)? {
            break;
        }

        match core.next_utterance()? {
            Capture::Speech(transcript) => {
                println!("transcript: {}", transcript.text());
            }
            Capture::NoSpeech => eprintln!("no speech"),
            Capture::Cancelled => break,
        }
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn config_from_env() -> Result<Config, Error> {
    let model = parse_model_path(env::var_os("TETO_WHISPER_MODEL"))?;
    let language = parse_language(env::var("TETO_WHISPER_LANGUAGE").ok())?;
    let use_gpu = parse_use_gpu(env::var("TETO_WHISPER_GPU").ok())?;
    Ok(Config {
        model,
        language,
        use_gpu,
    })
}

fn parse_model_path(value: Option<OsString>) -> Result<PathBuf, Error> {
    match value {
        None => Err(Error::Config("TETO_WHISPER_MODEL is required")),
        Some(path) if path.is_empty() => Err(Error::Config("TETO_WHISPER_MODEL must not be empty")),
        Some(path) => Ok(PathBuf::from(path)),
    }
}

fn parse_language(value: Option<String>) -> Result<Language, Error> {
    match value.as_deref() {
        None => Ok(Language::Code("en".into())),
        Some("") => Err(Error::Config("TETO_WHISPER_LANGUAGE must not be empty")),
        Some("auto") => Ok(Language::Auto),
        Some(code) => Ok(Language::Code(code.into())),
    }
}

fn parse_use_gpu(value: Option<String>) -> Result<bool, Error> {
    match value.as_deref() {
        None => Ok(cfg!(target_os = "macos")),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        Some(_) => Err(Error::Config(
            "TETO_WHISPER_GPU must be true, false, 1, or 0",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_command, parse_language, parse_model_path, parse_use_gpu};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use teto_core::{Error, Language};

    #[test]
    fn command_rejects_extra_arguments_before_side_effects() {
        assert!(
            parse_command(["journal-delete".into(), "/other/journal".into()].into_iter()).is_err()
        );
        assert_eq!(
            parse_command(["journal-init".into()].into_iter())
                .unwrap()
                .as_deref(),
            Some("journal-init")
        );
        assert!(parse_command(std::iter::empty()).unwrap().is_none());
    }

    #[test]
    fn model_path_requires_a_value() {
        assert!(matches!(parse_model_path(None), Err(Error::Config(_))));
    }

    #[test]
    fn model_path_rejects_empty_value() {
        assert!(matches!(
            parse_model_path(Some(OsString::new())),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn model_path_accepts_a_path() {
        let path = parse_model_path(Some(OsString::from("model.bin"))).unwrap();
        assert_eq!(path, PathBuf::from("model.bin"));
    }

    #[test]
    fn language_defaults_to_english() {
        assert!(matches!(
            parse_language(None),
            Ok(Language::Code(code)) if code == "en"
        ));
    }

    #[test]
    fn language_accepts_auto() {
        assert!(matches!(
            parse_language(Some("auto".into())),
            Ok(Language::Auto)
        ));
    }

    #[test]
    fn language_rejects_empty_value() {
        assert!(matches!(
            parse_language(Some(String::new())),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn use_gpu_parses_boolean_values() {
        assert!(parse_use_gpu(Some("true".into())).unwrap());
        assert!(!parse_use_gpu(Some("0".into())).unwrap());
        assert!(matches!(
            parse_use_gpu(Some("invalid".into())),
            Err(Error::Config(_))
        ));
    }
}
