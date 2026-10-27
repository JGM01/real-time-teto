//! Bounded diagnostics without clause text.
use crate::{
    span::{ByteSpan, IndexSpan},
    voicebank::id::PitchId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Substitution {
    TetoSchwa,
    VelarNasal,
    DarkL,
    Rhotic,
    DiphthongSplit,
    AffricateSplit,
    Glide,
    AliasForm,
    CoverageRepair,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct BankSubstitutions: u16 {
        const TETO_SCHWA = 1;
        const VELAR_NASAL = 2;
        const DARK_L = 4;
        const RHOTIC = 8;
        const COVERAGE_REPAIR = 16;
    }
}
impl BankSubstitutions {
    pub fn reasons(self) -> impl Iterator<Item = Substitution> {
        [
            (Self::TETO_SCHWA, Substitution::TetoSchwa),
            (Self::VELAR_NASAL, Substitution::VelarNasal),
            (Self::DARK_L, Substitution::DarkL),
            (Self::RHOTIC, Substitution::Rhotic),
            (Self::COVERAGE_REPAIR, Substitution::CoverageRepair),
        ]
        .into_iter()
        .filter_map(move |(flag, reason)| self.contains(flag).then_some(reason))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnostic {
    Oov {
        word: ByteSpan,
    },
    Ambiguous {
        word: ByteSpan,
        alternatives: u32,
    },
    UncertainSyllable {
        phones: IndexSpan,
    },
    Substitution {
        phones: IndexSpan,
        reason: Substitution,
    },
    PitchFallback {
        requested: PitchId,
        selected: PitchId,
    },
    MissingAlias {
        phones: IndexSpan,
        pitch: PitchId,
    },
}

/// Maximum retained diagnostic events.
pub const MAX_DIAGNOSTICS: usize = 128;

#[derive(Debug, Default)]
pub struct SpeechDiagnostics {
    counts: [u64; 6],
    events: Vec<Diagnostic>,
}

impl SpeechDiagnostics {
    pub fn record(&mut self, event: Diagnostic) {
        let kind = match event {
            Diagnostic::Oov { .. } => 0,
            Diagnostic::Ambiguous { .. } => 1,
            Diagnostic::UncertainSyllable { .. } => 2,
            Diagnostic::Substitution { .. } => 3,
            Diagnostic::PitchFallback { .. } => 4,
            Diagnostic::MissingAlias { .. } => 5,
        };
        self.counts[kind] = self.counts[kind].saturating_add(1);
        if self.events.len() < MAX_DIAGNOSTICS {
            self.events.push(event);
        }
    }

    pub fn events(&self) -> &[Diagnostic] {
        &self.events
    }
    pub fn counts(&self) -> &[u64; 6] {
        &self.counts
    }
    pub fn clear(&mut self) {
        self.counts = [0; 6];
        self.events.clear();
    }
}
