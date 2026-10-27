//! Local metadata and corpus qualification without audio output.
#[cfg(all(feature = "cmudict", feature = "voicebank-wav"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use teto_speech::{
        diagnostics::SpeechDiagnostics,
        input::{SpeechInput, SpeechKey},
        phone::BreakKind,
        plan::{PhoneTiming, ProsodyPlan, ProsodyPoint},
        text::phonemize::CmudictPhonemizer,
        voicebank::{cvvc::TetoCvvcPlanner, oto::import_voicebank},
    };
    let args: Vec<_> = std::env::args().collect();
    let path = args.get(1).ok_or("bank path required")?;
    let imported = import_voicebank(std::path::Path::new(path))?;
    let bank = imported.index;
    let oov = load_oov(args.get(3))?;
    let mut frontend = CmudictPhonemizer::embedded(oov)?;
    let planner = TetoCvvcPlanner::default();
    let mut diagnostics = SpeechDiagnostics::default();
    let corpus = args
        .get(2)
        .map(std::fs::read_to_string)
        .transpose()?
        .unwrap_or_else(|| "Hello world.\n".into());
    let mut records = Vec::new();
    let mut failed = 0;
    for (index, text) in corpus.lines().filter(|s| !s.is_empty()).enumerate() {
        diagnostics.clear();
        let input = SpeechInput::new(SpeechKey::new(0, 1, 0)?, text)?;
        let result=frontend.phonemize(&input,&mut diagnostics).and_then(|sequence| {
            let timings=sequence.phones().iter().enumerate().map(|(i,_)|PhoneTiming {start_us:i as u64*100_000,end_us:(i as u64+1)*100_000}).collect();
            let prosody=ProsodyPlan::new(&sequence,sequence.phones().len() as u64*100_000,timings,vec![ProsodyPoint {at_us:0,f0_millihz:Some(220_000),gain_q16:65536}])?;
            let plan=planner.plan(&sequence,prosody,&bank,&mut diagnostics)?;
            plan.validate(&sequence,&bank)?;
            Ok(serde_json::json!({"case":index,"phones":sequence.phones().iter().map(|p|p.phone().symbol()).collect::<Vec<_>>(),
                "words":sequence.words().iter().map(|w|vec![w.phones.start(),w.phones.end()]).collect::<Vec<_>>(),
                "breaks":sequence.breaks().filter(|b|b.kind!=BreakKind::Word).map(|b|b.after_phone).collect::<Vec<_>>(),
                "aliases":plan.units().iter().map(|u|u.alias(&bank).and_then(|id|bank.alias_text(id))).collect::<Vec<_>>(),"groups":plan.groups().iter().map(|g|plan.units()[g.range()].iter().map(|u|u.alias(&bank).and_then(|id|bank.alias_text(id))).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "reference_groups":plan.groups().iter().map(|g|plan.units()[g.range()].iter().filter(|u|!u.substitutions.contains(teto_speech::diagnostics::BankSubstitutions::COVERAGE_REPAIR)).map(|u|u.alias(&bank).and_then(|id|bank.alias_text(id))).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "repairs":plan.units().iter().filter(|u|u.substitutions.contains(teto_speech::diagnostics::BankSubstitutions::COVERAGE_REPAIR)).map(|u|vec![u.target_phones.start(),u.target_phones.end()]).collect::<Vec<_>>(),
                "unit_phones":plan.units().iter().map(|u|vec![u.target_phones.start(),u.target_phones.end()]).collect::<Vec<_>>(),
                "unit_repairs":plan.units().iter().map(|u|u.substitutions.contains(teto_speech::diagnostics::BankSubstitutions::COVERAGE_REPAIR)).collect::<Vec<_>>(),
                "units":plan.units().len(),"diagnostics":diagnostics.counts()}))
        });
        match result {
            Ok(record) => records.push(record),
            Err(error) => {
                failed += 1;
                records.push(serde_json::json!({"case":index,"error":format!("{error:?}")}));
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({"sources":imported.report.sources,"oto_rows":imported.report.rows,"aliases":imported.report.aliases,"duplicates":imported.report.duplicate_rows,"missing_preferred_aliases":imported.report.missing_preferred_aliases,"unsupported_aliases":imported.report.unsupported_aliases,"revision":bank.revision(),"cases":records.len(),"failed":failed,"records":records})
    );
    if failed > 0 {
        return Err("corpus qualification failed".into());
    }
    Ok(())
}
#[cfg(all(feature = "cmudict", feature = "voicebank-wav"))]
fn load_oov(
    _path: Option<&String>,
) -> teto_speech::error::Result<teto_speech::text::phonemize::OovBackend> {
    #[cfg(feature = "g2p-gru")]
    if let Some(path) = _path {
        return Ok(teto_speech::text::phonemize::OovBackend::Gru(Box::new(
            teto_speech::text::gru::G2pEnGru::load(std::path::Path::new(path))?,
        )));
    }
    Ok(teto_speech::text::phonemize::OovBackend::Disabled)
}
#[cfg(not(all(feature = "cmudict", feature = "voicebank-wav")))]
fn main() {
    eprintln!("enable cmudict and voicebank-wav");
}
