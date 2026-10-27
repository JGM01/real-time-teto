use crate::{
    journal_setup,
    llm_client::{Client, io_error, require},
};
use std::{
    env,
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use teto_llm_protocol::*;

pub(super) fn run(mode: &str, shutdown: &Arc<AtomicBool>) -> Result<()> {
    if !matches!(mode, "text" | "voice") {
        return Err(failure(ErrorKind::Config));
    }
    let mut client = open_client(shutdown)?;
    let result = if mode == "text" {
        text_loop(&mut client, shutdown)
    } else {
        voice_loop(&mut client, shutdown)
    };
    let closed = client.close();
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
    };
    let mut client = Client::spawn(Arc::clone(shutdown))?;
    require(client.request(&Request::Open(config))?, |r| {
        matches!(r, Response::Ready)
    })?;
    Ok(client)
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
    first_clause_ms: Option<f64>,
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
    let mut first_clause_ms = None;
    let id = client.start(input)?;
    let mut playback = PlaybackStop::Complete;
    let mut error = None;
    loop {
        match client.request(&Request::Pull(id)) {
            Ok(Response::Event(ClauseEvent::Clause(clause))) if clause.turn() == id => {
                first_clause_ms.get_or_insert(started.elapsed().as_secs_f64() * 1000.0);
                if shutdown.load(Ordering::Acquire) {
                    client.cancel(id)?;
                    playback = PlaybackStop::Interrupted;
                    break;
                }
                let mut output = io::stdout().lock();
                if let Err(failure) =
                    writeln!(output, "clause: {}", clause.text()).and_then(|()| output.flush())
                {
                    client.cancel(id)?;
                    playback = PlaybackStop::Failed;
                    error = Some(io_error(failure));
                    break;
                }
                require(
                    client.request(&Request::Played(ClausePlayed {
                        turn: id,
                        index: clause.index(),
                    }))?,
                    |r| matches!(r, Response::Acknowledged),
                )?;
                if interrupt {
                    client.cancel(id)?;
                    playback = PlaybackStop::Interrupted;
                    break;
                }
            }
            Ok(Response::Event(ClauseEvent::End { turn, .. })) if turn == id => break,
            Ok(Response::Event(ClauseEvent::Cancelled { turn })) if turn == id => {
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
    let record = client.finish(id, playback)?;
    report_turn(&record);
    error.map_or(
        Ok(RenderedTurn {
            record,
            first_clause_ms,
        }),
        Err,
    )
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
        generated_tokens = record.metrics.generated_tokens, first_answer_ms = record.metrics.first_answer_ms,
        first_clause_ms = record.metrics.first_clause_ms, clauses = record.metrics.clauses, partial = record.partial,
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
        let load_ms = started.elapsed().as_secs_f64() * 1000.0;
        let rendered = render(
            &mut client,
            UserInput::new("Say a friendly greeting.".into(), false)?,
            shutdown,
        )?;
        if shutdown.load(Ordering::Acquire) {
            client.close()?;
            break;
        }
        let clause_ms = rendered
            .first_clause_ms
            .ok_or_else(|| failure(ErrorKind::Generation))?;
        samples.push(load_ms + clause_ms);
        eprintln!(
            "{}",
            serde_json::json!({"cold_process_sample": index + 1, "load_ms": load_ms,
            "first_clause_from_launch_ms": load_ms + clause_ms, "first_clause_after_load_ms": clause_ms,
            "metrics": rendered.record.metrics, "asr_resident": true, "audio": "pending"})
        );
        client.close()?;
    }
    samples.sort_by(f64::total_cmp);
    if samples.len() == policy::COLD_START_RUNS {
        eprintln!(
            "{}",
            serde_json::json!({"cold_process_summary": true, "samples": samples.len(),
            "p50_ms": samples[policy::COLD_START_RUNS / 2 - 1],
            "p95_ms": samples[(policy::COLD_START_RUNS * 95).div_ceil(100) - 1],
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
    let mut client = open_client(shutdown)?;
    let started = std::time::Instant::now();
    let mut samples = Vec::with_capacity(policy::WARM_TURNS);
    let mut turns = 0u64;
    let prompts = [
        "My name is Robin. Greet me in one sentence.",
        "What name did I give you?",
        "Briefly explain why the sky looks blue.",
        "I meant sunsets. Explain the correction briefly.",
        "In one sentence describe a dozen apples using English words.",
        "Give a short plain English greeting.",
    ];
    while turns < policy::WARM_TURNS as u64 + 1
        || started.elapsed().as_secs() < policy::SOAK_SECONDS
    {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let prompt = prompts[turns as usize % prompts.len()];
        let interrupt = lifecycle && (turns + 1) % policy::SOAK_INTERRUPT_EVERY == 0;
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
        if turns > 0 && samples.len() < policy::WARM_TURNS {
            samples.push(
                rendered
                    .first_clause_ms
                    .ok_or_else(|| failure(ErrorKind::Generation))?,
            );
        }
        turns += 1;
        let line = serde_json::json!({"qualification_turn": turns, "metrics": record.metrics,
            "profile": record.profile, "policy": policy::VERSION,
            "cache": record.metrics.cache_mode, "first_clause_at_parent_ms": rendered.first_clause_ms,
            "generation_stop": record.generation_stop, "playback_stop": record.playback_stop,
            "partial": record.partial, "lifecycle": lifecycle, "audio_qualification": "pending", "asr_resident": true});
        eprintln!("{line}");
        if lifecycle {
            client = lifecycle_transition(client, turns, shutdown)?;
        }
    }
    report_qualification(&mut samples, turns, started);
    client.close()
}

fn report_qualification(samples: &mut [f64], turns: u64, started: std::time::Instant) {
    samples.sort_by(f64::total_cmp);
    if samples.len() == policy::WARM_TURNS {
        eprintln!(
            "{}",
            serde_json::json!({"qualification_summary": true, "turns": turns,
            "elapsed_seconds": started.elapsed().as_secs(), "first_clause_p50_ms": samples[49],
            "first_clause_p95_ms": samples[94], "audio": "pending",
            "status": if started.elapsed().as_secs() >= policy::SOAK_SECONDS { "measured" } else { "interrupted" }})
        );
    }
}

fn lifecycle_transition(
    mut client: Client,
    turns: u64,
    shutdown: &Arc<AtomicBool>,
) -> Result<Client> {
    if turns % policy::SOAK_REOPEN_EVERY == 0 {
        client.close()?;
        client = open_client(shutdown)?;
        eprintln!(
            "{}",
            serde_json::json!({"qualification_action": "clean_process_reopen", "after_turn": turns})
        );
    } else if turns % policy::SOAK_RESET_EVERY == 0 {
        require(client.request(&Request::Reset)?, |response| {
            matches!(response, Response::Reset)
        })?;
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
