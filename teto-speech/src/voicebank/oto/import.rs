//! Metadata-only local bank import.
use super::visit_rows;
use crate::{
    error::{Result, SpeechError},
    limits::*,
    types::*,
    voicebank::{
        alias::BankSymbol,
        manifest::PitchMapping,
        read::{BankReader, safe_path},
    },
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Debug, Default)]
pub struct ImportReport {
    pub sources: u32,
    pub rows: u32,
    pub aliases: u32,
    pub duplicate_rows: u32,
    pub unsupported_aliases: u32,
    pub missing_preferred_aliases: u32,
    pub missing_aliases: Vec<(PitchId, &'static str)>,
    pub duplicate_lines: Vec<u32>,
    pub unsupported_ids: Vec<AliasId>,
}
#[derive(Debug)]
pub enum BankLocation {
    Directory {
        root: std::path::PathBuf,
    },
    Archive {
        path: std::path::PathBuf,
        prefix: String,
    },
}
pub struct ImportedBank {
    pub location: BankLocation,
    pub manifest: VoicebankManifest,
    pub report: ImportReport,
}
pub struct TetoOtoImport;
struct Row {
    alias: String,
    oto: OtoEntry,
}
impl TetoOtoImport {
    pub fn load(path: &Path) -> Result<ImportedBank> {
        let mut reader = BankReader::open(path)?;
        let entries = reader.entries();
        let expanded = entries.iter().try_fold(0u64, |n, (_, s)| {
            n.checked_add(*s).ok_or(SpeechError::Bounds)
        })?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(SpeechError::Bounds);
        }
        let otos: Vec<_> = entries
            .iter()
            .filter(|(p, _)| p.ends_with("oto.ini") && p.rsplit('/').next() == Some("oto.ini"))
            .collect();
        if otos.is_empty() || otos.len() > MAX_PITCHES {
            return Err(SpeechError::VoicebankInvalid);
        }
        let root = common_root(&otos.iter().map(|e| e.0.as_str()).collect::<Vec<_>>());
        let mut bank = VoicebankManifest {
            revision: [0; 32],
            inventory: AliasInventoryId([0; 32]),
            archive_sha256: reader.digest(),
            alias_text: String::new(),
            aliases: Vec::new(),
            symbols: Vec::new(),
            oto: Vec::new(),
            sources: Vec::new(),
            pitches: Vec::new(),
        };
        let mut rows = Vec::new();
        let mut source_ids = BTreeMap::new();
        for (path, _) in otos {
            import_oto(
                &mut reader,
                &entries,
                path,
                &root,
                &mut bank,
                &mut rows,
                &mut source_ids,
            )?;
        }
        let mut report = finish_aliases(&mut bank, rows)?;
        bank.validate()?;
        hash_manifest(&mut bank);
        report_inventory(&bank, &mut report);
        let location = if path.is_dir() {
            BankLocation::Directory {
                root: path.canonicalize()?.join(root.trim_end_matches('/')),
            }
        } else {
            BankLocation::Archive {
                path: path.canonicalize()?,
                prefix: root,
            }
        };
        Ok(ImportedBank {
            manifest: bank,
            report,
            location,
        })
    }
}
fn report_inventory(bank: &VoicebankManifest, report: &mut ImportReport) {
    report.sources = bank.sources.len() as u32;
    report.rows = bank.oto.len() as u32;
    report.aliases = bank.aliases.len() as u32;
    for group in &bank.pitches {
        for alias in ["- bV", "bV", "N g", "N g-", "N k", "N k-"] {
            if bank.find(alias, group.id).is_none() {
                report.missing_preferred_aliases += 1;
                if report.missing_aliases.len() < MAX_DIAGNOSTICS {
                    report.missing_aliases.push((group.id, alias));
                }
            }
        }
    }
}
fn common_root(paths: &[&str]) -> String {
    let first: Vec<_> = paths[0].split('/').collect();
    let mut count = first.len().saturating_sub(1);
    for path in paths.iter().skip(1) {
        count = count.min(
            first
                .iter()
                .zip(path.split('/'))
                .take_while(|(a, b)| **a == *b)
                .count(),
        );
    }
    if count == 0 {
        String::new()
    } else {
        format!("{}/", first[..count].join("/"))
    }
}
fn decode(bytes: &[u8]) -> Result<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.trim_start_matches('\u{feff}').to_owned());
    }
    let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
    if bad {
        return Err(SpeechError::VoicebankInvalid);
    }
    Ok(text.into_owned())
}
fn import_oto(
    reader: &mut BankReader,
    entries: &[(String, u64)],
    path: &str,
    root: &str,
    bank: &mut VoicebankManifest,
    rows: &mut Vec<Row>,
    sources: &mut BTreeMap<String, SourceId>,
) -> Result<()> {
    let bytes = reader.bytes(path, MAX_OTO_BYTES)?;
    let directory = path
        .strip_suffix("oto.ini")
        .ok_or(SpeechError::VoicebankInvalid)?;
    let pitches = import_pitch(reader, entries, directory, root, bank)?;
    visit_rows(&bytes, |line_number, row| {
        if rows.len() >= MAX_OTO_ROWS {
            return Err(SpeechError::Bounds);
        }
        let pitch = *pitches
            .iter()
            .find(|id| bank.core_alias(row.alias, **id).is_some())
            .ok_or(SpeechError::VoicebankInvalid)?;
        let source_path = format!("{directory}{}", safe_path(row.source)?);
        let source = match sources.get(&source_path) {
            Some(id) => *id,
            None => {
                if bank.sources.len() >= MAX_SOURCES {
                    return Err(SpeechError::Bounds);
                }
                let length = entries
                    .iter()
                    .find(|e| e.0 == source_path)
                    .ok_or(SpeechError::VoicebankInvalid)?
                    .1;
                let id = SourceId(bank.sources.len() as u32);
                bank.sources
                    .push(read_wav(reader, &source_path, root, length)?);
                sources.insert(source_path, id);
                id
            }
        };
        let duration = bank.sources[source.0 as usize]
            .frames
            .checked_mul(1_000_000)
            .ok_or(SpeechError::Bounds)?
            / 44100;
        let end_us = if row.cutoff_us < 0 {
            row.offset_us.checked_add(row.cutoff_us.unsigned_abs())
        } else {
            duration.checked_sub(row.cutoff_us as u64)
        }
        .ok_or(SpeechError::OtoTiming { line: line_number })?;
        rows.push(Row {
            alias: row.alias.into(),
            oto: OtoEntry {
                alias: AliasId(0),
                source,
                line: line_number,
                offset_us: row.offset_us,
                consonant_us: row.consonant_us,
                consonant_velocity_percent: None,
                preutterance_us: row.preutterance_us,
                overlap_us: row.overlap_us,
                cutoff_us: row.cutoff_us,
                end_us,
                pitch,
            },
        });
        Ok(())
    })
}
fn import_pitch(
    reader: &mut BankReader,
    entries: &[(String, u64)],
    directory: &str,
    root: &str,
    bank: &mut VoicebankManifest,
) -> Result<Vec<PitchId>> {
    let mut mappings = Vec::new();
    let prefix = format!("{directory}prefix.map");
    if entries.iter().any(|e| e.0 == prefix) {
        for line in decode(&reader.bytes(&prefix, MAX_OTO_BYTES)?)?.lines() {
            let parts: Vec<_> = line.split('\t').collect();
            if parts.len() != 3 || mappings.len() >= 128 || line.len() > MAX_LINE_BYTES {
                return Err(SpeechError::VoicebankInvalid);
            }
            mappings.push(PitchMapping {
                note: parts[0].into(),
                prefix: parts[1].into(),
                suffix: parts[2].into(),
            });
        }
    }
    mappings.sort_by(|a, b| {
        a.prefix
            .cmp(&b.prefix)
            .then(a.suffix.cmp(&b.suffix))
            .then(a.note.cmp(&b.note))
    });
    let mut groups = Vec::<Vec<PitchMapping>>::new();
    for mapping in mappings {
        if groups
            .last()
            .and_then(|g| g.first())
            .is_none_or(|m| m.prefix != mapping.prefix || m.suffix != mapping.suffix)
        {
            groups.push(Vec::new());
        }
        groups
            .last_mut()
            .ok_or(SpeechError::VoicebankInvalid)?
            .push(mapping);
    }
    if groups.is_empty() {
        groups.push(Vec::new());
    }
    let mut ids = Vec::new();
    for mappings in groups {
        if bank.pitches.len() >= MAX_PITCHES {
            return Err(SpeechError::Bounds);
        }
        let id = PitchId(bank.pitches.len() as u32);
        bank.pitches.push(PitchGroup {
            id,
            directory: directory.strip_prefix(root).unwrap_or(directory).into(),
            mappings,
        });
        ids.push(id);
    }
    ids.sort_by_key(|id| {
        std::cmp::Reverse(
            bank.pitches[id.0 as usize]
                .mappings
                .first()
                .map(|m| m.prefix.len() + m.suffix.len())
                .unwrap_or(0),
        )
    });
    Ok(ids)
}
fn read_wav(reader: &mut BankReader, path: &str, root: &str, length_bytes: u64) -> Result<Source> {
    let input = reader.reader(path)?.take(MAX_WAV_HEADER_BYTES);
    let wav = hound::WavReader::new(input).map_err(|_| SpeechError::VoicebankInvalid)?;
    let spec = wav.spec();
    if spec.sample_rate != 44100
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(SpeechError::UnsupportedFormat);
    }
    let frames = u64::from(wav.duration());
    let data_offset_bytes = MAX_WAV_HEADER_BYTES - wav.into_inner().limit();
    let data_length_bytes = frames.checked_mul(2).ok_or(SpeechError::Bounds)?;
    if frames == 0
        || data_offset_bytes
            .checked_add(data_length_bytes)
            .is_none_or(|end| end > length_bytes)
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    Ok(Source {
        path: path.strip_prefix(root).unwrap_or(path).into(),
        length_bytes,
        sample_rate_hz: 44100,
        channels: 1,
        bits_per_sample: 16,
        frames,
        data_offset_bytes,
        data_length_bytes,
    })
}
fn finish_aliases(bank: &mut VoicebankManifest, mut rows: Vec<Row>) -> Result<ImportReport> {
    rows.sort_by(|a, b| {
        a.alias
            .cmp(&b.alias)
            .then(a.oto.pitch.0.cmp(&b.oto.pitch.0))
            .then(a.oto.line.cmp(&b.oto.line))
    });
    let mut report = ImportReport::default();
    for row in rows {
        let duplicate = bank
            .aliases
            .last()
            .is_some_and(|a| bank.alias_text[a.text.range()] == row.alias);
        let id = if duplicate {
            let alias = bank
                .aliases
                .last_mut()
                .ok_or(SpeechError::VoicebankInvalid)?;
            if bank.oto.last().is_some_and(|o| o.pitch == row.oto.pitch) {
                report.duplicate_rows += 1;
                if report.duplicate_lines.len() < MAX_DIAGNOSTICS {
                    report.duplicate_lines.push(row.oto.line);
                }
            }
            alias.units.end += 1;
            alias.id
        } else {
            let id = AliasId(bank.aliases.len() as u32);
            let text_start = bank.alias_text.len() as u32;
            bank.alias_text.push_str(&row.alias);
            let symbol_start = bank.symbols.len() as u32;
            let core = bank
                .core_alias(&row.alias, row.oto.pitch)
                .ok_or(SpeechError::VoicebankInvalid)?;
            if !BankSymbol::parse_alias(core, &mut bank.symbols) {
                bank.symbols.truncate(symbol_start as usize);
                report.unsupported_aliases += 1;
                if report.unsupported_ids.len() < MAX_DIAGNOSTICS {
                    report.unsupported_ids.push(id);
                }
            }
            bank.aliases.push(Alias {
                id,
                text: IndexSpan::new(text_start, bank.alias_text.len() as u32)?,
                symbols: IndexSpan::new(symbol_start, bank.symbols.len() as u32)?,
                units: IndexSpan::new(bank.oto.len() as u32, bank.oto.len() as u32 + 1)?,
            });
            id
        };
        bank.oto.push(OtoEntry {
            alias: id,
            ..row.oto
        });
    }
    Ok(report)
}
fn hash_manifest(bank: &mut VoicebankManifest) {
    let mut inventory = Sha256::new();
    inventory.update(b"teto-alias-inventory-v1");
    inventory.update((bank.aliases.len() as u64).to_le_bytes());
    for alias in &bank.aliases {
        hash_bytes(
            &mut inventory,
            bank.alias_text[alias.text.range()].as_bytes(),
        );
    }
    bank.inventory = AliasInventoryId(inventory.finalize().into());
    let mut hash = Sha256::new();
    hash.update(b"teto-manifest-v1");
    hash.update(bank.inventory.0);
    hash.update((bank.sources.len() as u64).to_le_bytes());
    for s in &bank.sources {
        hash_bytes(&mut hash, s.path.as_bytes());
        for value in [
            s.length_bytes,
            s.frames,
            s.data_offset_bytes,
            s.data_length_bytes,
        ] {
            hash.update(value.to_le_bytes());
        }
    }
    hash.update((bank.pitches.len() as u64).to_le_bytes());
    for p in &bank.pitches {
        hash_bytes(&mut hash, p.directory.as_bytes());
        hash.update((p.mappings.len() as u64).to_le_bytes());
        for m in &p.mappings {
            for s in [&m.note, &m.prefix, &m.suffix] {
                hash_bytes(&mut hash, s.as_bytes());
            }
        }
    }
    hash.update((bank.oto.len() as u64).to_le_bytes());
    for r in &bank.oto {
        for value in [
            r.alias.0 as u64,
            r.source.0 as u64,
            r.line as u64,
            r.offset_us,
            r.consonant_us,
            r.consonant_velocity_percent
                .map(|v| u64::from(v.get()))
                .unwrap_or(0),
            r.preutterance_us,
            r.overlap_us as u64,
            r.cutoff_us as u64,
            r.end_us,
            r.pitch.0 as u64,
        ] {
            hash.update(value.to_le_bytes());
        }
    }
    bank.revision = hash.finalize().into();
}
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
