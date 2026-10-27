//! Explicit real-model measurement harness. It never renders or simulates audio.
use crate::*;
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Audio metrics remain pending until the actual output node exists.
pub const AUDIO_STATUS: &str = "pending: real TTS required";
/// At most one record per completed turn is written; response text is excluded.
pub fn measure(
    config: OpenConfig,
    output: &mut impl Write,
    stop: &AtomicBool,
    minimum_turns: usize,
    duration: Duration,
) -> Result<()> {
    if minimum_turns == 0
        || minimum_turns > policy::WARM_TURNS
        || duration > Duration::from_secs(policy::SOAK_SECONDS)
    {
        return Err(failure(ErrorKind::Config));
    }
    let loaded = Instant::now();
    let profile = config.profile;
    let mut llm = Llm::open(config)?;
    let load_ms = loaded.elapsed().as_secs_f64() * 1000.0;
    cancellation_probe(&mut llm, output)?;
    let started = Instant::now();
    let mut turns = 0usize;
    while turns < minimum_turns || started.elapsed() < duration {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let prompt = [
            "My name is Robin. Greet me briefly.",
            "What name did I give you?",
            "Explain a rainbow in one sentence.",
            "I meant a sunset. Correct your explanation briefly.",
        ][turns % 4];
        let mut turn = llm.start_turn(UserInput::new(prompt.into(), false)?)?;
        let playback = receive(&mut turn, stop)?;
        let record = llm.finish_turn(&mut turn, playback, ReceiptSource::TestSink)?;
        let line = serde_json::json!({"turn": record.turn.get(), "profile": profile, "runtime": policy::RUNTIME,
            "policy": policy::VERSION, "load_ms": load_ms, "metrics": record.metrics,
            "generation_stop": record.generation_stop, "playback_stop": record.playback_stop,
            "partial": record.partial, "audio": AUDIO_STATUS, "cache": record.metrics.cache_mode, "asr_resident": false});
        writeln!(output, "{line}")
            .and_then(|()| output.flush())
            .map_err(|_| failure(ErrorKind::Worker))?;
        turns = turns
            .checked_add(1)
            .ok_or_else(|| failure(ErrorKind::Worker))?;
    }
    llm.close()?;
    writeln!(
        output,
        "{}",
        serde_json::json!({"measurement_summary": true,
        "measured_turns": turns, "elapsed_seconds": started.elapsed().as_secs_f64(),
        "minimum_turns": minimum_turns, "requested_seconds": duration.as_secs(),
        "interrupted": stop.load(Ordering::Acquire), "audio": AUDIO_STATUS})
    )
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
fn receive(turn: &mut Turn, stop: &AtomicBool) -> Result<PlaybackStop> {
    loop {
        if stop.load(Ordering::Acquire) {
            turn.control().cancel();
            return Ok(PlaybackStop::Interrupted);
        }
        match turn.next_clause()? {
            ClauseEvent::Clause(clause) => {
                // This sink exercises receipts only. It is never proof of speech or playback.
                turn.acknowledge(ClausePlayed {
                    turn: clause.turn(),
                    index: clause.index(),
                })?;
            }
            ClauseEvent::End { .. } => return Ok(PlaybackStop::Complete),
            ClauseEvent::Cancelled { .. } => return Ok(PlaybackStop::Interrupted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualification_marks_audio_pending() {
        assert_eq!(AUDIO_STATUS, "pending: real TTS required");
    }
    #[test]
    fn qualification_requires_real_tts_evidence() {
        assert!(AUDIO_STATUS.contains("real TTS"));
        assert!(!AUDIO_STATUS.contains("passed"));
    }
}
