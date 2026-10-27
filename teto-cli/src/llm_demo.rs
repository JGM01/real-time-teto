use crate::journal_setup;
use std::{
    env,
    io::{self, Write},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use teto_llm_client::{Client, ClientError, ClientOptions, WorkerLaunch};
use teto_llm_protocol::{measurement, *};

pub(super) fn run(mode: crate::Mode, shutdown: &Arc<AtomicBool>) -> Result<()> {
    let mut client = open_client(shutdown)?;
    let result = match mode {
        crate::Mode::Text => text_loop(&mut client, shutdown),
        crate::Mode::Voice => voice_loop(&mut client, shutdown),
        crate::Mode::Transcribe => Err(failure(ErrorKind::Config)),
    };
    let closed = client.close().map_err(ClientError::into_protocol);
    result.and(closed)
}

fn open_client(shutdown: &Arc<AtomicBool>) -> Result<Client> {
    let config = OpenConfig {
        assets: env::var_os("TETO_LLM_ASSETS")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| failure(ErrorKind::Config))?
            .into(),
        profile: ProfileId::parse(
            env::var("TETO_LLM_PROFILE")
                .as_deref()
                .unwrap_or(policy::PROFILE_ID),
        )?,
        output_language: OutputLanguage::English,
        system_prompt: env::var("TETO_LLM_SYSTEM_PROMPT").unwrap_or_default(),
        seed: 0,
        journal: journal_setup::access()?,
        asset_cache: Some(asset_cache_directory()),
        host: host_identity(),
        delivery_window: DeliveryWindow::One,
    };
    let launch = WorkerLaunch::from_environment().map_err(ClientError::into_protocol)?;
    let options = ClientOptions {
        shutdown: Some(Arc::clone(shutdown)),
        ..Default::default()
    };
    Client::open(launch, config, options).map_err(ClientError::into_protocol)
}

fn host_identity() -> HostIdentity {
    #[cfg(target_os = "macos")]
    {
        let device = command_output("/usr/sbin/sysctl", &["-n", "hw.model"]);
        let os = command_output("/usr/bin/sw_vers", &["-productVersion"]);
        HostIdentity::new(device.unwrap_or_default(), os.unwrap_or_default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        HostIdentity::new("unknown", "unknown")
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn asset_cache_directory() -> std::path::PathBuf {
    if let Some(path) = env::var_os("TETO_ASSET_CACHE") {
        return path.into();
    }
    let directory = env::temp_dir().join("teto-asset-cache");
    let _ = std::fs::create_dir_all(&directory);
    directory
}

fn text_loop(client: &mut Client, shutdown: &Arc<AtomicBool>) -> Result<()> {
    let mut reader = TextInput::default();
    while let Some(text) = reader.next(shutdown)? {
        if text.trim().is_empty() {
            continue;
        }
        render(client, UserInput::new(text, false)?, shutdown)?;
    }
    Ok(())
}
fn voice_loop(client: &mut Client, shutdown: &Arc<AtomicBool>) -> Result<()> {
    let config = crate::config_from_env().map_err(|_| failure(ErrorKind::Config))?;
    let mut core = teto_core::TetoCore::open(config, Arc::clone(shutdown))
        .map_err(|_| failure(ErrorKind::Worker))?;
    let result = voice_captures(client, &mut core, shutdown);
    let close = core.close().map_err(|_| failure(ErrorKind::Worker));
    result.and(close)
}
fn voice_captures(
    client: &mut Client,
    core: &mut teto_core::TetoCore,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    while !shutdown.load(Ordering::Acquire) {
        eprintln!("Press SPACE to activate, Ctrl-C to quit.");
        if !crate::terminal::wait_for_space(shutdown).map_err(io_error)? {
            break;
        }
        match core
            .next_utterance()
            .map_err(|_| failure(ErrorKind::Worker))?
        {
            teto_core::Capture::Speech(transcript) => {
                writeln!(io::stdout(), "transcript: {}", transcript.text()).map_err(io_error)?;
                render(
                    client,
                    UserInput::new(transcript.text().into(), transcript.is_truncated())?,
                    shutdown,
                )?;
            }
            teto_core::Capture::NoSpeech => eprintln!("no speech"),
            teto_core::Capture::Cancelled => break,
        }
    }
    Ok(())
}
struct RenderedTurn {
    record: FinalizedTurn,
    first_clause_us: Option<u64>,
}
fn render(client: &mut Client, input: UserInput, shutdown: &AtomicBool) -> Result<RenderedTurn> {
    render_until(client, input, shutdown, false)
}
fn render_until(
    client: &mut Client,
    input: UserInput,
    shutdown: &AtomicBool,
    interrupt: bool,
) -> Result<RenderedTurn> {
    let started = std::time::Instant::now();
    let mut first_clause_us = None;
    let id = client.start(input).map_err(ClientError::into_protocol)?;
    let mut playback = PlaybackStop::Complete;
    let mut error = None;
    loop {
        match client.pull(id).map_err(ClientError::into_protocol) {
            Ok(ClauseEvent::Clause(clause)) if clause.turn() == id => {
                first_clause_us.get_or_insert(
                    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                if shutdown.load(Ordering::Acquire) {
                    client.cancel(id).map_err(ClientError::into_protocol)?;
                    playback = PlaybackStop::Interrupted;
                    break;
                }
                let mut output = io::stdout().lock();
                if let Err(failure) =
                    writeln!(output, "clause: {}", clause.text()).and_then(|()| output.flush())
                {
                    client.cancel(id).map_err(ClientError::into_protocol)?;
                    playback = PlaybackStop::Failed;
                    error = Some(io_error(failure));
                    break;
                }
                client
                    .acknowledge(ClausePlayed {
                        turn: id,
                        index: clause.index(),
                    })
                    .map_err(ClientError::into_protocol)?;
                if interrupt {
                    client.cancel(id).map_err(ClientError::into_protocol)?;
                    playback = PlaybackStop::Interrupted;
                    break;
                }
            }
            Ok(ClauseEvent::End { turn, .. }) if turn == id => break,
            Ok(ClauseEvent::Cancelled { turn }) if turn == id => {
                playback = PlaybackStop::Interrupted;
                break;
            }
            Ok(_) => {
                error = Some(failure(ErrorKind::Protocol));
                playback = PlaybackStop::Failed;
                break;
            }
            Err(failure) => {
                playback = playback_after_stream_error(failure.kind());
                error = Some(failure);
                break;
            }
        }
    }
    let record = client
        .finish(id, playback, ReceiptSource::Display)
        .map_err(ClientError::into_protocol)?;
    report_turn(&record);
    error.map_or(
        Ok(RenderedTurn {
            record,
            first_clause_us,
        }),
        Err,
    )
}

fn io_error(_: io::Error) -> Error {
    failure(ErrorKind::Worker)
}

fn playback_after_stream_error(kind: ErrorKind) -> PlaybackStop {
    match kind {
        ErrorKind::Generation | ErrorKind::Normalization => PlaybackStop::Complete,
        _ => PlaybackStop::Failed,
    }
}

fn report_turn(record: &FinalizedTurn) {
    eprintln!(
        "turn: {} generation={:?} playback={:?} truncated={} partial={}",
        record.turn.get(),
        record.generation_stop,
        record.playback_stop,
        record.user.truncated(),
        record.partial
    );
    tracing::info!(target: "teto_cli::llm", turn = record.turn.get(), prompt_tokens = record.metrics.prompt_tokens,
        generated_tokens = record.metrics.generated_tokens, first_answer_us = record.metrics.first_answer_us,
        first_clause_us = record.metrics.first_clause_us, clauses = record.metrics.clauses, partial = record.partial,
        generation_stop = ?record.generation_stop, playback_stop = ?record.playback_stop, "turn");
}

/// Real child inference with ASR engines resident. No TTS/audio claims.
pub(super) fn qualify(shutdown: &Arc<AtomicBool>) -> Result<()> {
    with_asr(shutdown, qualify_resident)
}
pub(super) fn qualify_cold(shutdown: &Arc<AtomicBool>) -> Result<()> {
    with_asr(shutdown, cold_resident)
}
pub(super) fn qualify_lifecycle(shutdown: &Arc<AtomicBool>) -> Result<()> {
    with_asr(shutdown, lifecycle_resident)
}
fn with_asr(
    shutdown: &Arc<AtomicBool>,
    operation: fn(&Arc<AtomicBool>) -> Result<()>,
) -> Result<()> {
    let config = crate::config_from_env().map_err(|_| failure(ErrorKind::Config))?;
    let core = teto_core::TetoCore::open(config, Arc::clone(shutdown))
        .map_err(|_| failure(ErrorKind::Worker))?;
    let result = operation(shutdown);
    let close = core.close().map_err(|_| failure(ErrorKind::Worker));
    result.and(close)
}
fn cold_resident(shutdown: &Arc<AtomicBool>) -> Result<()> {
    let mut samples = Vec::with_capacity(policy::COLD_START_RUNS);
    for index in 0..policy::COLD_START_RUNS {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let started = std::time::Instant::now();
        let mut client = open_client(shutdown)?;
        let load_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let rendered = render(
            &mut client,
            UserInput::new("Say a friendly greeting.".into(), false)?,
            shutdown,
        )?;
        if shutdown.load(Ordering::Acquire) {
            client.close().map_err(ClientError::into_protocol)?;
            break;
        }
        let clause_us = rendered
            .first_clause_us
            .ok_or_else(|| failure(ErrorKind::Generation))?;
        samples.push(load_us.saturating_add(clause_us));
        eprintln!(
            "{}",
            serde_json::json!({"cold_process_sample": index + 1, "load_us": load_us,
            "first_clause_from_launch_us": load_us.saturating_add(clause_us),
            "first_clause_after_load_us": clause_us,
            "metrics": rendered.record.metrics, "asr_resident": true, "audio": "pending"})
        );
        client.close().map_err(ClientError::into_protocol)?;
    }
    if samples.len() == policy::COLD_START_RUNS {
        eprintln!(
            "{}",
            serde_json::json!({"cold_process_summary": true, "samples": samples.len(),
            "p50_us": measurement::percentile(&samples, 50),
            "p95_us": measurement::percentile(&samples, 95),
            "definition": "fresh worker/model/context; OS page and shader caches uncontrolled", "audio": "pending"})
        );
    }
    Ok(())
}
fn qualify_resident(shutdown: &Arc<AtomicBool>) -> Result<()> {
    qualify_run(shutdown, false)
}
fn lifecycle_resident(shutdown: &Arc<AtomicBool>) -> Result<()> {
    qualify_run(shutdown, true)
}
fn qualify_run(shutdown: &Arc<AtomicBool>, lifecycle: bool) -> Result<()> {
    let options = if lifecycle {
        measurement::Options::lifecycle()
    } else {
        measurement::Options::default()
    };
    let mut client = open_client(shutdown)?;
    let started = std::time::Instant::now();
    let mut warm = Vec::with_capacity(policy::WARM_TURNS);
    let mut all = Vec::with_capacity(policy::WARM_TURNS);
    let mut cache_hits = Vec::new();
    let mut rebuilds = 0u32;
    let mut evictions = 0u32;
    let mut turns = 0u64;
    while turns < options.minimum_turns as u64 + 1 || started.elapsed() < options.duration {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let prompt = measurement::PROMPTS[turns as usize % measurement::PROMPTS.len()];
        let interrupt = options
            .interrupt_every
            .is_some_and(|period| (turns + 1).is_multiple_of(period));
        let rendered = render_until(
            &mut client,
            UserInput::new(prompt.into(), false)?,
            shutdown,
            interrupt,
        )?;
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let record = rendered.record;
        let first_clause_us = rendered
            .first_clause_us
            .ok_or_else(|| failure(ErrorKind::Generation))?;
        if turns > 0 && warm.len() < policy::WARM_TURNS {
            warm.push(first_clause_us);
        }
        all.push(first_clause_us);
        if record.metrics.cached_prefix_tokens > 0 {
            cache_hits.push(first_clause_us);
        }
        rebuilds = rebuilds.saturating_add(record.metrics.context_rebuilds);
        evictions = evictions.saturating_add(record.metrics.evicted_exchanges);
        turns = turns.saturating_add(1);
        let line = serde_json::json!({"qualification_turn": turns, "metrics": record.metrics,
            "profile": record.profile, "policy": policy::VERSION,
            "cache": record.metrics.cache_mode, "first_clause_at_parent_us": first_clause_us,
            "generation_stop": record.generation_stop, "playback_stop": record.playback_stop,
            "partial": record.partial, "lifecycle": lifecycle, "audio": measurement::AUDIO_STATUS, "asr_resident": true});
        eprintln!("{line}");
        if lifecycle {
            client = lifecycle_transition(client, turns, shutdown)?;
        }
    }
    report_qualification(
        &warm,
        &all,
        &cache_hits,
        turns,
        rebuilds,
        evictions,
        started,
    );
    client.close().map_err(ClientError::into_protocol)
}

fn report_qualification(
    warm: &[u64],
    all: &[u64],
    cache_hits: &[u64],
    turns: u64,
    rebuilds: u32,
    evictions: u32,
    started: std::time::Instant,
) {
    if warm.len() < policy::WARM_TURNS {
        return;
    }
    eprintln!(
        "{}",
        serde_json::json!({"qualification_summary": true, "turns": turns,
        "elapsed_seconds": started.elapsed().as_secs(),
        "first_clause_p50_us": measurement::percentile(all, 50),
        "first_clause_p95_us": measurement::percentile(all, 95),
        "warm_p50_us": measurement::percentile(warm, 50),
        "warm_p95_us": measurement::percentile(warm, 95),
        "cache_hit_turns": cache_hits.len(),
        "cache_hit_p95_us": measurement::percentile(cache_hits, 95),
        "context_rebuilds": rebuilds,
        "evicted_exchanges": evictions,
        "audio": measurement::AUDIO_STATUS,
        "status": if started.elapsed().as_secs() >= policy::SOAK_SECONDS { "measured" } else { "interrupted" }})
    );
}

fn lifecycle_transition(
    mut client: Client,
    turns: u64,
    shutdown: &Arc<AtomicBool>,
) -> Result<Client> {
    if turns.is_multiple_of(policy::SOAK_REOPEN_EVERY) {
        client.close().map_err(ClientError::into_protocol)?;
        client = open_client(shutdown)?;
        eprintln!(
            "{}",
            serde_json::json!({"qualification_action": "clean_process_reopen", "after_turn": turns})
        );
    } else if turns.is_multiple_of(policy::SOAK_RESET_EVERY) {
        client.reset().map_err(ClientError::into_protocol)?;
        eprintln!(
            "{}",
            serde_json::json!({"qualification_action": "reset", "after_turn": turns})
        );
    }
    Ok(client)
}

#[derive(Default)]
struct TextInput {
    buffer: Vec<u8>,
    ended: bool,
}
impl TextInput {
    fn next(&mut self, shutdown: &AtomicBool) -> Result<Option<String>> {
        loop {
            if shutdown.load(Ordering::Acquire) {
                return Ok(None);
            }
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                return line_text(line).map(Some);
            }
            if self.ended {
                return if self.buffer.is_empty() {
                    Ok(None)
                } else {
                    line_text(std::mem::take(&mut self.buffer)).map(Some)
                };
            }
            if self.buffer.len() > policy::INPUT_BYTES {
                return Err(failure(ErrorKind::Config));
            }
            if !stdin_ready()? {
                continue;
            }
            let mut bytes = [0; 1024];
            let Some(count) = read_stdin(&mut bytes)? else {
                continue;
            };
            self.ended = count == 0;
            self.buffer.extend_from_slice(&bytes[..count]);
        }
    }
}
#[cfg(unix)]
fn read_stdin(bytes: &mut [u8]) -> Result<Option<usize>> {
    match rustix::io::read(io::stdin(), bytes) {
        Ok(count) => Ok(Some(count)),
        Err(rustix::io::Errno::INTR) => Ok(None),
        Err(_) => Err(failure(ErrorKind::Worker)),
    }
}
#[cfg(not(unix))]
fn read_stdin(_: &mut [u8]) -> Result<Option<usize>> {
    Err(failure(ErrorKind::Config))
}
fn line_text(mut bytes: Vec<u8>) -> Result<String> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if bytes.len() > policy::INPUT_BYTES {
        return Err(failure(ErrorKind::Config));
    }
    String::from_utf8(bytes).map_err(|_| failure(ErrorKind::Config))
}
#[cfg(unix)]
fn stdin_ready() -> Result<bool> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let input = io::stdin();
    let mut descriptors = [PollFd::new(&input, PollFlags::IN)];
    poll(
        &mut descriptors,
        Some(&Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        }),
    )
    .map(|ready| ready > 0)
    .or_else(|error| {
        if error == rustix::io::Errno::INTR {
            Ok(false)
        } else {
            Err(failure(ErrorKind::Worker))
        }
    })
}
#[cfg(not(unix))]
fn stdin_ready() -> Result<bool> {
    Err(failure(ErrorKind::Config))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generation_failure_keeps_completed_display_independent() {
        assert_eq!(
            playback_after_stream_error(ErrorKind::Generation),
            PlaybackStop::Complete
        );
        assert_eq!(
            playback_after_stream_error(ErrorKind::Normalization),
            PlaybackStop::Complete
        );
        assert_eq!(
            playback_after_stream_error(ErrorKind::Worker),
            PlaybackStop::Failed
        );
        assert_eq!(
            playback_after_stream_error(ErrorKind::Protocol),
            PlaybackStop::Failed
        );
    }
}
