//! Measure prompt prefill on one long input. Requires the desktop model.
use std::{path::PathBuf, time::Instant};
use teto_llm::*;

fn main() -> Result<()> {
    let assets = std::env::var_os("TETO_LLM_ASSETS").expect("set TETO_LLM_ASSETS");
    let config = OpenConfig {
        assets: PathBuf::from(assets),
        profile: ProfileId::Desktop,
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 0,
        journal: JournalAccess::Disabled,
        asset_cache: std::env::var_os("TETO_ASSET_CACHE").map(Into::into),
        host: HostIdentity::new("Mac15,7", "27.0.1"),
        delivery_window: DeliveryWindow::One,
    };
    let mut llm = Llm::open(config)?;
    let sentence = "The quiet river passes the green valley and the distant mountain. ";
    let text = sentence.repeat(180);
    let started = Instant::now();
    let mut turn = llm.start_turn(UserInput::new(text, false)?)?;
    let _ = turn.next_clause()?;
    let record = llm.finish_turn(
        &mut turn,
        PlaybackStop::Interrupted,
        ReceiptSource::TestSink,
    )?;
    let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let metrics = &record.metrics;
    let tokens_per_second = if metrics.prefill_us == 0 {
        0.0
    } else {
        f64::from(metrics.evaluated_prompt_tokens) * 1_000_000.0 / metrics.prefill_us as f64
    };
    println!(
        "{}",
        serde_json::json!({
            "prompt_tokens": metrics.prompt_tokens,
            "evaluated_prompt_tokens": metrics.evaluated_prompt_tokens,
            "prefill_us": metrics.prefill_us,
            "prefill_tokens_per_second": tokens_per_second,
            "first_clause_us": metrics.first_clause_us,
            "elapsed_us": elapsed_us,
            "cache_reason": metrics.cache_reason,
        })
    );
    llm.close()
}
