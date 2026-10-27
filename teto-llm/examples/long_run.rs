//! Measure a short multi-turn run and the low-water eviction path.
use std::{path::PathBuf, time::Instant};
use teto_llm::*;

fn main() -> Result<()> {
    let assets = std::env::var_os("TETO_LLM_ASSETS").expect("set TETO_LLM_ASSETS");
    let config = OpenConfig {
        assets: PathBuf::from(assets),
        profile: ProfileId::Desktop,
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 7,
        journal: JournalAccess::Disabled,
        asset_cache: std::env::var_os("TETO_ASSET_CACHE").map(Into::into),
        host: HostIdentity::new("Mac15,7", "27.0.1"),
        delivery_window: DeliveryWindow::One,
    };
    let mut llm = Llm::open(config)?;
    let prompts = [
        "My name is Robin. Greet me briefly.",
        "What name did I give you?",
        "Explain a rainbow in one sentence.",
        "I meant a sunset. Correct your explanation briefly.",
    ];
    let started = Instant::now();
    let mut samples = Vec::new();
    let mut rebuilds = 0u32;
    let mut evictions = 0u32;
    for index in 0..120u64 {
        let prompt = prompts[index as usize % prompts.len()];
        let mut turn = llm.start_turn(UserInput::new(prompt.into(), false)?)?;
        loop {
            match turn.next_clause()? {
                ClauseEvent::Clause(clause) => {
                    turn.acknowledge(ClausePlayed {
                        turn: clause.turn(),
                        index: clause.index(),
                    })?;
                }
                ClauseEvent::End { .. } | ClauseEvent::Cancelled { .. } => break,
            }
        }
        let record = llm.finish_turn(&mut turn, PlaybackStop::Complete, ReceiptSource::TestSink)?;
        if let Some(first_clause_us) = record.metrics.first_clause_us {
            samples.push(first_clause_us);
        }
        rebuilds = rebuilds.saturating_add(record.metrics.context_rebuilds);
        evictions = evictions.saturating_add(record.metrics.evicted_exchanges);
    }
    llm.close()?;
    println!(
        "{}",
        serde_json::json!({
            "turns": samples.len(),
            "elapsed_seconds": started.elapsed().as_secs_f64(),
            "first_clause_p50_us": measurement::percentile(&samples, 50),
            "first_clause_p95_us": measurement::percentile(&samples, 95),
            "context_rebuilds": rebuilds,
            "evicted_exchanges": evictions,
        })
    );
    Ok(())
}
