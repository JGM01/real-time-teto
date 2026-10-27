//! Imported source ranges, OTO rows, and deterministic alias lookup.
use crate::{
    error::{Result, SpeechError},
    span::IndexSpan,
    voicebank::id::{AliasIndex, AliasInventoryId, OtoRowId, PitchId, SourceId},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: IndexSpan,
    pub length_bytes: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub frames: u64,
    pub data_offset_bytes: u64,
    pub data_length_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub text: IndexSpan,
    pub symbols: IndexSpan,
    pub units: IndexSpan,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchMapping {
    pub note: String,
    pub prefix: String,
    pub suffix: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PitchGroup {
    pub directory: String,
    pub mappings: Vec<PitchMapping>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtoEntry {
    pub alias: AliasIndex,
    pub source: SourceId,
    pub line: u32,
    pub offset_us: u64,
    pub consonant_us: u64,
    pub consonant_velocity_percent: Option<std::num::NonZeroU16>,
    pub preutterance_us: u64,
    pub overlap_us: i64,
    pub cutoff_us: i64,
    pub end_us: u64,
    pub pitch: PitchId,
}
#[derive(Debug)]
pub struct VoicebankIndex {
    pub(crate) revision: [u8; 32],
    pub(crate) inventory: AliasInventoryId,
    pub(crate) archive_sha256: Option<[u8; 32]>,
    pub(crate) alias_text: String,
    pub(crate) source_text: String,
    pub(crate) aliases: Vec<Alias>,
    pub(crate) symbols: Vec<super::alias::BankSymbol>,
    pub(crate) oto: Vec<OtoEntry>,
    pub(crate) sources: Vec<Source>,
    pub(crate) pitches: Vec<PitchGroup>,
}
impl VoicebankIndex {
    pub fn revision(&self) -> [u8; 32] {
        self.revision
    }
    pub fn inventory(&self) -> AliasInventoryId {
        self.inventory
    }
    pub fn archive_sha256(&self) -> Option<[u8; 32]> {
        self.archive_sha256
    }
    pub fn aliases(&self) -> &[Alias] {
        &self.aliases
    }
    pub fn oto(&self) -> &[OtoEntry] {
        &self.oto
    }
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }
    /// Return one source path.
    pub fn source_path(&self, id: SourceId) -> Option<&str> {
        let source = self.sources.get(id.0 as usize)?;
        self.source_text.get(source.path.range())
    }
    pub fn pitches(&self) -> &[PitchGroup] {
        &self.pitches
    }
    pub fn alias_text(&self, id: AliasIndex) -> Option<&str> {
        self.aliases
            .get(id.0 as usize)
            .and_then(|a| self.alias_text.get(a.text.range()))
    }
    pub fn alias_symbols(&self, id: AliasIndex) -> Option<&[super::alias::BankSymbol]> {
        self.aliases
            .get(id.0 as usize)
            .and_then(|a| self.symbols.get(a.symbols.range()))
    }
    pub fn find(&self, text: &str, pitch: PitchId) -> Option<(OtoRowId, &OtoEntry)> {
        if let Some(found) = self.find_exact(text, pitch) {
            return Some(found);
        }
        let mapping = self.pitches.get(pitch.0 as usize)?.mappings.first()?;
        use std::fmt::Write;
        let mut decorated =
            arrayvec::ArrayString::<{ crate::voicebank::limits::MAX_ALIAS_BYTES }>::new();
        write!(decorated, "{}{text}{}", mapping.prefix, mapping.suffix).ok()?;
        self.find_exact(&decorated, pitch)
    }
    #[cfg(feature = "voicebank-wav")]
    pub(crate) fn core_alias<'a>(&self, text: &'a str, pitch: PitchId) -> Option<&'a str> {
        let group = self.pitches.get(pitch.0 as usize)?;
        match group.mappings.first() {
            Some(mapping) => text
                .strip_prefix(&mapping.prefix)?
                .strip_suffix(&mapping.suffix),
            None => Some(text),
        }
    }
    fn find_exact(&self, text: &str, pitch: PitchId) -> Option<(OtoRowId, &OtoEntry)> {
        let index = self
            .aliases
            .binary_search_by(|a| self.alias_text[a.text.range()].cmp(text))
            .ok()?;
        let alias = &self.aliases[index];
        if alias.symbols.is_empty() {
            return None;
        }
        let span = alias.units;
        self.oto[span.range()]
            .iter()
            .enumerate()
            .find(|(_, row)| row.pitch == pitch)
            .map(|(i, row)| (OtoRowId(span.start + i as u32), row))
    }
    pub fn validate(&self) -> Result<()> {
        if self.aliases.is_empty()
            || self.oto.is_empty()
            || self.sources.is_empty()
            || self.aliases.len() > crate::voicebank::limits::MAX_OTO_ROWS
            || self.sources.len() > crate::voicebank::limits::MAX_SOURCES
            || self.oto.len() > crate::voicebank::limits::MAX_OTO_ROWS
            || self.pitches.is_empty()
            || self.pitches.len() > crate::voicebank::limits::MAX_PITCHES
        {
            return Err(SpeechError::LimitExceeded);
        }
        let mut previous_text: Option<&str> = None;
        for (i, alias) in self.aliases.iter().enumerate() {
            let text = self
                .alias_text
                .get(alias.text.range())
                .ok_or(SpeechError::VoicebankInvalid)?;
            let id = AliasIndex(i as u32);
            if previous_text.is_some_and(|previous| previous >= text)
                || self.symbols.get(alias.symbols.range()).is_none()
                || self.symbols[alias.symbols.range()].is_empty()
                || self
                    .oto
                    .get(alias.units.range())
                    .is_none_or(|rows| rows.iter().any(|row| row.alias != id))
            {
                return Err(SpeechError::VoicebankInvalid);
            }
            previous_text = Some(text);
        }
        for row in &self.oto {
            let source = self
                .sources
                .get(row.source.0 as usize)
                .ok_or(SpeechError::VoicebankInvalid)?;
            if self.source_text.get(source.path.range()).is_none() {
                return Err(SpeechError::VoicebankInvalid);
            }
            let duration_us = source
                .frames
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_div(u64::from(source.sample_rate_hz)))
                .ok_or(SpeechError::VoicebankInvalid)?;
            if row.alias.0 as usize >= self.aliases.len()
                || row.pitch.0 as usize >= self.pitches.len()
                || row.offset_us >= row.end_us
                || row.end_us > duration_us
                || row.consonant_us > row.end_us - row.offset_us
                || row.preutterance_us > row.end_us - row.offset_us
                || row.overlap_us.unsigned_abs() > duration_us
            {
                return Err(SpeechError::OtoTiming { line: row.line });
            }
        }
        Ok(())
    }
}
