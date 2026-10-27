//! Synthetic metadata for property tests and fuzz targets.
use crate::{
    error::{Result, SpeechError},
    limits::*,
    types::*,
    voicebank::alias::BankSymbol,
};
pub fn manifest(alias_names: &[&str]) -> Result<VoicebankManifest> {
    if alias_names.len() > MAX_OTO_ROWS {
        return Err(SpeechError::Bounds);
    }
    let mut names = alias_names.to_vec();
    names.sort();
    names.dedup();
    let mut bank = VoicebankManifest {
        revision: [0; 32],
        inventory: AliasInventoryId([0; 32]),
        archive_sha256: None,
        alias_text: String::new(),
        aliases: Vec::new(),
        symbols: Vec::new(),
        oto: Vec::new(),
        sources: vec![Source {
            path: "synthetic.wav".into(),
            length_bytes: 8864,
            sample_rate_hz: 44100,
            channels: 1,
            bits_per_sample: 16,
            frames: 4410,
            data_offset_bytes: 44,
            data_length_bytes: 8820,
        }],
        pitches: vec![PitchGroup {
            id: PitchId(0),
            directory: String::new(),
            mappings: Vec::new(),
        }],
    };
    for name in names {
        if name.is_empty() || name.len() > MAX_ALIAS_BYTES {
            return Err(SpeechError::Bounds);
        }
        let id = AliasId(bank.aliases.len() as u32);
        let text = bank.alias_text.len() as u32;
        let symbols = bank.symbols.len() as u32;
        bank.alias_text.push_str(name);
        if !BankSymbol::parse_alias(name, &mut bank.symbols) {
            return Err(SpeechError::VoicebankInvalid);
        }
        bank.aliases.push(Alias {
            id,
            text: IndexSpan::new(text, bank.alias_text.len() as u32)?,
            symbols: IndexSpan::new(symbols, bank.symbols.len() as u32)?,
            units: IndexSpan::new(id.0, id.0 + 1)?,
        });
        bank.oto.push(OtoEntry {
            alias: id,
            source: SourceId(0),
            line: id.0 + 1,
            offset_us: 0,
            consonant_us: 10000,
            consonant_velocity_percent: None,
            preutterance_us: 5000,
            overlap_us: 0,
            cutoff_us: -100000,
            end_us: 100000,
            pitch: PitchId(0),
        });
    }
    bank.validate()?;
    Ok(bank)
}
