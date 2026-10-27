//! Real-model measurement harness. It never renders or simulates audio.
use crate::*;
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub use teto_llm_protocol::measurement::{AUDIO_STATUS, Options, PROMPTS, percentile};

/// Run one measurement and write JSON lines to `output`.
pub fn measure(
    config: OpenConfig,
    output: &mut impl Write,
    stop: &AtomicBool,
    minimum_turns: usize,
    duration: Duration,
) -> Result<()> {
    measure_with_options(
        config,
        output,
        stop,
        Options {
            minimum_turns,
            duration,
            ..Options::default()
        },
    )
}

/// Run one measurement with lifecycle actions.
pub fn measure_with_options(
    config: OpenConfig,
    output: &mut impl Write,
    stop: &AtomicBool,
    options: Options,
) -> Result<()> {
    if options.minimum_turns == 0
        || options.minimum_turns > policy::WARM_TURNS
        || options.duration > Duration::from_secs(policy::SOAK_SECONDS)
    {
        return Err(failure(ErrorKind::Config));
    }
    let loaded = Instant::now();
    let profile = config.profile;
    let mut llm = Llm::open(config.clone())?;
    let load_us = elapsed_us(loaded);
    cancellation_probe(&mut llm, output)?;
    let started = Instant::now();
    let mut turns = 0u64;
    let mut samples = Vec::with_capacity(policy::WARM_TURNS);
    while turns < options.minimum_turns as u64 || started.elapsed() < options.duration {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let prompt = PROMPTS[turns as usize % PROMPTS.len()];
        let interrupt = options
            .interrupt_every
            .is_some_and(|period| (turns + 1) % period == 0);
        let turn_started = Instant::now();
        let mut turn = llm.start_turn(UserInput::new(prompt.into(), false)?)?;
        let (playback, first_clause_us) = receive(&mut turn, stop, interrupt, turn_started)?;
        let record = llm.finish_turn(&mut turn, playback, ReceiptSource::TestSink)?;
        if turns > 0 && samples.len() < policy::WARM_TURNS {
            samples.push(first_clause_us);
        }
        turns = turns.saturating_add(1);
        write_turn(
            output,
            turns,
            profile,
            load_us,
            first_clause_us,
            &record,
            options.asr_resident,
        )?;
        if let Some(period) = options.reopen_every
            && turns % period == 0
        {
            llm.close()?;
            llm = Llm::open(config.clone())?;
            action(output, "clean_process_reopen", turns)?;
        } else if let Some(period) = options.reset_every
            && turns % period == 0
        {
            llm.reset_conversation()?;
            action(output, "reset", turns)?;
        }
    }
    llm.close()?;
    write_summary(output, &samples, turns, options, stop, started)
}

fn write_turn(
    output: &mut impl Write,
    turns: u64,
    profile: ProfileId,
    load_us: u64,
    first_clause_us: u64,
    record: &FinalizedTurn,
    asr_resident: bool,
) -> Result<()> {
    let line = serde_json::json!({
        "qualification_turn": turns,
        "profile": profile.as_str(),
        "runtime": policy::RUNTIME,
        "policy": policy::VERSION,
        "load_us": load_us,
        "metrics": record.metrics,
        "first_clause_at_parent_us": first_clause_us,
        "generation_stop": record.generation_stop,
        "playback_stop": record.playback_stop,
        "partial": record.partial,
        "asr_resident": asr_resident,
        "audio": AUDIO_STATUS,
    });
    writeln!(output, "{line}")
        .and_then(|()| output.flush())
        .map_err(|_| failure(ErrorKind::Worker))
}

fn write_summary(
    output: &mut impl Write,
    samples: &[u64],
    turns: u64,
    options: Options,
    stop: &AtomicBool,
    started: Instant,
) -> Result<()> {
    let summary = serde_json::json!({
        "qualification_summary": true,
        "measured_turns": turns,
        "elapsed_seconds": started.elapsed().as_secs_f64(),
        "minimum_turns": options.minimum_turns,
        "requested_seconds": options.duration.as_secs(),
        "interrupted": stop.load(Ordering::Acquire),
        "first_clause_p50_us": percentile(samples, 50),
        "first_clause_p95_us": percentile(samples, 95),
        "audio": AUDIO_STATUS,
    });
    writeln!(output, "{summary}")
        .and_then(|()| output.flush())
        .map_err(|_| failure(ErrorKind::Worker))
}

fn action(output: &mut impl Write, name: &str, after_turn: u64) -> Result<()> {
    let line = serde_json::json!({"qualification_action": name, "after_turn": after_turn});
    writeln!(output, "{line}")
        .and_then(|()| output.flush())
        .map_err(|_| failure(ErrorKind::Worker))
}

fn cancellation_probe(llm: &mut Llm, output: &mut impl Write) -> Result<()> {
    let mut turn = llm.start_turn(UserInput::new(
        "Explain a forest in many complete sentences.".into(),
        false,
    )?)?;
    turn.control().cancel();
    let record = llm.finish_turn(
        &mut turn,
        PlaybackStop::Interrupted,
        ReceiptSource::TestSink,
    )?;
    if record.generation_stop != GenerationStop::Cancelled {
        return Err(failure(ErrorKind::Generation));
    }
    writeln!(
        output,
        "{}",
        serde_json::json!({"cancellation_probe": true,
        "metrics": record.metrics, "generation_stop": record.generation_stop,
        "playback_stop": record.playback_stop, "partial": record.partial,
        "audio": AUDIO_STATUS})
    )
    .and_then(|()| output.flush())
    .map_err(|_| failure(ErrorKind::Worker))
}

fn receive(
    turn: &mut Turn,
    stop: &AtomicBool,
    interrupt: bool,
    started: Instant,
) -> Result<(PlaybackStop, u64)> {
    let mut first_clause_us = None;
    loop {
        if stop.load(Ordering::Acquire) {
            turn.control().cancel();
            return Ok((
                PlaybackStop::Interrupted,
                first_clause_us.unwrap_or_else(|| elapsed_us(started)),
            ));
        }
        match turn.next_clause()? {
            ClauseEvent::Clause(clause) => {
                first_clause_us.get_or_insert_with(|| elapsed_us(started));
                turn.acknowledge(ClausePlayed {
                    turn: clause.turn(),
                    index: clause.index(),
                })?;
                if interrupt {
                    turn.control().cancel();
                    return Ok((
                        PlaybackStop::Interrupted,
                        first_clause_us.unwrap_or_default(),
                    ));
                }
            }
            ClauseEvent::End { .. } => {
                return Ok((
                    PlaybackStop::Complete,
                    first_clause_us.unwrap_or_else(|| elapsed_us(started)),
                ));
            }
            ClauseEvent::Cancelled { .. } => {
                return Ok((
                    PlaybackStop::Interrupted,
                    first_clause_us.unwrap_or_else(|| elapsed_us(started)),
                ));
            }
        }
    }
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}
