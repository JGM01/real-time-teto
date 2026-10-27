//! Imported source ranges, OTO rows, and deterministic alias lookup.
use crate::{
    error::{Result, SpeechError},
    types::{AliasId, AliasInventoryId, IndexSpan, PitchId, UnitId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u32);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: String,
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
    pub id: AliasId,
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
    pub id: PitchId,
    pub directory: String,
    pub mappings: Vec<PitchMapping>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtoEntry {
    pub alias: AliasId,
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
pub struct VoicebankManifest {
    pub(crate) revision: [u8; 32],
    pub(crate) inventory: AliasInventoryId,
    pub(crate) archive_sha256: Option<[u8; 32]>,
    pub(crate) alias_text: String,
    pub(crate) aliases: Vec<Alias>,
    pub(crate) symbols: Vec<super::alias::BankSymbol>,
    pub(crate) oto: Vec<OtoEntry>,
    pub(crate) sources: Vec<Source>,
    pub(crate) pitches: Vec<PitchGroup>,
}
impl VoicebankManifest {
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
    pub fn pitches(&self) -> &[PitchGroup] {
        &self.pitches
    }
    pub fn alias_text(&self, id: AliasId) -> Option<&str> {
        self.aliases
            .get(id.0 as usize)
            .and_then(|a| self.alias_text.get(a.text.range()))
    }
    pub fn alias_symbols(&self, id: AliasId) -> Option<&[super::alias::BankSymbol]> {
        self.aliases
            .get(id.0 as usize)
            .and_then(|a| self.symbols.get(a.symbols.range()))
    }
    pub fn find(&self, text: &str, pitch: PitchId) -> Option<(UnitId, &OtoEntry)> {
        if let Some(found) = self.find_exact(text, pitch) {
            return Some(found);
        }
        let mapping = self.pitches.get(pitch.0 as usize)?.mappings.first()?;
        use std::fmt::Write;
        let mut decorated = arrayvec::ArrayString::<{ crate::limits::MAX_ALIAS_BYTES }>::new();
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
    fn find_exact(&self, text: &str, pitch: PitchId) -> Option<(UnitId, &OtoEntry)> {
        let index = self
            .aliases
            .binary_search_by(|a| self.alias_text[a.text.range()].cmp(text))
            .ok()?;
        let span = self.aliases[index].units;
        self.oto[span.range()]
            .iter()
            .enumerate()
            .find(|(_, row)| row.pitch == pitch)
            .map(|(i, row)| (UnitId(span.start + i as u32), row))
    }
    pub fn validate(&self) -> Result<()> {
        if self.aliases.is_empty()
            || self.oto.is_empty()
            || self.sources.is_empty()
            || self.aliases.len() > crate::limits::MAX_OTO_ROWS
            || self.sources.len() > crate::limits::MAX_SOURCES
            || self.oto.len() > crate::limits::MAX_OTO_ROWS
            || self.pitches.is_empty()
            || self.pitches.len() > crate::limits::MAX_PITCHES
        {
            return Err(SpeechError::Bounds);
        }
        for (i, alias) in self.aliases.iter().enumerate() {
            if alias.id.0 as usize != i
                || self.alias_text.get(alias.text.range()).is_none()
                || self.symbols.get(alias.symbols.range()).is_none()
                || self.oto.get(alias.units.range()).is_none()
            {
                return Err(SpeechError::VoicebankInvalid);
            }
        }
        for row in &self.oto {
            let source = self
                .sources
                .get(row.source.0 as usize)
                .ok_or(SpeechError::VoicebankInvalid)?;
            if row.alias.0 as usize >= self.aliases.len()
                || row.pitch.0 as usize >= self.pitches.len()
                || row.offset_us >= row.end_us
                || row.end_us > source.frames.saturating_mul(1_000_000) / 44100
                || row.consonant_us > row.end_us - row.offset_us
                || row.preutterance_us > row.end_us - row.offset_us
                || row.overlap_us.unsigned_abs() > source.frames.saturating_mul(1_000_000) / 44100
            {
                return Err(SpeechError::OtoTiming { line: row.line });
            }
        }
        Ok(())
    }
}
