//! Checked target timing and complete selected-unit coverage.
use crate::{
    diagnostics::BankSubstitutions,
    error::{Result, SpeechError},
    input::SpeechKey,
    sequence::PhonemeSequence,
    span::{IndexSpan, check_span},
    text::limits::MAX_PHONES,
    voicebank::{
        alias::BankSymbol,
        id::{AliasIndex, OtoRowId, PitchId},
        index::VoicebankIndex,
    },
};

/// Maximum realization symbols.
pub const MAX_REALIZATIONS: usize = 2 * MAX_PHONES;
/// Maximum selected units.
pub const MAX_UNITS: usize = 2048;
/// Maximum candidate probes.
pub const MAX_CANDIDATE_PROBES: usize = 4096;
/// Maximum pitch points.
pub const MAX_CONTOUR_POINTS: usize = 1024;
/// Maximum gain in Q16.
pub const MAX_GAIN_Q16: u32 = 4 * 65_536;

/// One phone timing on the target timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhoneTiming {
    pub start_us: u64,
    pub end_us: u64,
}

/// One F0 and gain point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProsodyPoint {
    pub at_us: u64,
    pub f0_millihz: Option<u32>,
    pub gain_q16: u32,
}
#[derive(Debug)]
pub struct ProsodyPlan {
    key: SpeechKey,
    sequence_revision: u64,
    duration_us: u64,
    timings: Vec<PhoneTiming>,
    contour: Vec<ProsodyPoint>,
}
impl ProsodyPlan {
    pub fn new(
        sequence: &PhonemeSequence,
        duration_us: u64,
        timings: Vec<PhoneTiming>,
        contour: Vec<ProsodyPoint>,
    ) -> Result<Self> {
        if timings.len() != sequence.phones.len() || contour.len() > MAX_CONTOUR_POINTS {
            return Err(SpeechError::LimitExceeded);
        }
        let mut end = 0;
        for timing in &timings {
            if timing.start_us < end
                || timing.end_us <= timing.start_us
                || timing.end_us > duration_us
            {
                return Err(SpeechError::InvalidSpan);
            }
            end = timing.end_us;
        }
        let mut previous = None;
        for p in &contour {
            if p.at_us > duration_us
                || p.f0_millihz == Some(0)
                || p.gain_q16 > MAX_GAIN_Q16
                || previous.is_some_and(|t| t >= p.at_us)
            {
                return Err(SpeechError::InvalidSpan);
            }
            previous = Some(p.at_us);
        }
        Ok(Self {
            key: sequence.input.key(),
            sequence_revision: sequence.revision(),
            duration_us,
            timings,
            contour,
        })
    }
    pub fn key(&self) -> SpeechKey {
        self.key
    }
    pub fn sequence_revision(&self) -> u64 {
        self.sequence_revision
    }
    pub fn duration_us(&self) -> u64 {
        self.duration_us
    }
    pub fn timings(&self) -> &[PhoneTiming] {
        &self.timings
    }
    pub fn contour(&self) -> &[ProsodyPoint] {
        &self.contour
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhoneSymbol {
    pub target_phone: u32,
    pub symbol: BankSymbol,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedUnit {
    pub oto: OtoRowId,
    pub target_phones: IndexSpan,
    pub realizations: IndexSpan,
    pub substitutions: BankSubstitutions,
}

impl PlannedUnit {
    /// Return the selected alias.
    pub fn alias(&self, bank: &VoicebankIndex) -> Option<AliasIndex> {
        bank.oto.get(self.oto.0 as usize).map(|row| row.alias)
    }

    /// Return the selected pitch group.
    pub fn pitch(&self, bank: &VoicebankIndex) -> Option<PitchId> {
        bank.oto.get(self.oto.0 as usize).map(|row| row.pitch)
    }

    /// Return the first target phone start time.
    pub fn start_us(&self, prosody: &ProsodyPlan) -> u64 {
        prosody.timings[self.target_phones.start as usize].start_us
    }

    /// Return the last target phone end time.
    pub fn end_us(&self, prosody: &ProsodyPlan) -> u64 {
        prosody.timings[self.target_phones.end as usize - 1].end_us
    }
}
#[derive(Debug)]
pub struct UnitPlan {
    pub(crate) key: SpeechKey,
    pub(crate) sequence_revision: u64,
    pub(crate) bank_revision: [u8; 32],
    pub(crate) units: Vec<PlannedUnit>,
    pub(crate) groups: Vec<IndexSpan>,
    pub(crate) realizations: Vec<PhoneSymbol>,
    pub(crate) prosody: ProsodyPlan,
}
impl UnitPlan {
    pub fn key(&self) -> SpeechKey {
        self.key
    }
    pub fn sequence_revision(&self) -> u64 {
        self.sequence_revision
    }
    pub fn bank_revision(&self) -> [u8; 32] {
        self.bank_revision
    }
    pub fn units(&self) -> &[PlannedUnit] {
        &self.units
    }
    pub fn groups(&self) -> &[IndexSpan] {
        &self.groups
    }
    pub fn realizations(&self) -> &[PhoneSymbol] {
        &self.realizations
    }
    pub fn prosody(&self) -> &ProsodyPlan {
        &self.prosody
    }
    pub fn validate(&self, sequence: &PhonemeSequence, bank: &VoicebankIndex) -> Result<()> {
        if self.key != sequence.input.key()
            || self.sequence_revision != sequence.revision()
            || self.prosody.sequence_revision != self.sequence_revision
            || self.bank_revision != bank.revision
            || self.prosody.timings.len() != sequence.phones.len()
        {
            return Err(SpeechError::InvalidInput);
        }
        if self.units.is_empty()
            || self.units.len() > MAX_UNITS
            || self.realizations.len() > MAX_REALIZATIONS
        {
            return Err(SpeechError::LimitExceeded);
        }
        self.validate_realizations(sequence)?;
        let mut covered = [0u16; MAX_REALIZATIONS];
        for unit in &self.units {
            check_span(unit.realizations, self.realizations.len())?;
            for count in &mut covered[unit.realizations.range()] {
                *count += 1;
            }
        }
        let mut group_end = 0;
        for group in &self.groups {
            check_span(*group, self.units.len())?;
            if group.start != group_end {
                return Err(SpeechError::InvalidSpan);
            }
            group_end = group.end;
        }
        if group_end as usize != self.units.len() {
            return Err(SpeechError::InvalidSpan);
        }
        for unit in &self.units {
            self.validate_unit(unit, sequence, bank)?;
            if unit
                .substitutions
                .contains(BankSubstitutions::COVERAGE_REPAIR)
                && !covered[unit.realizations.range()].contains(&1)
            {
                return Err(SpeechError::InvalidSpan);
            }
        }
        if covered[..self.realizations.len()].contains(&0) {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
    fn validate_unit(
        &self,
        unit: &PlannedUnit,
        sequence: &PhonemeSequence,
        bank: &VoicebankIndex,
    ) -> Result<()> {
        check_span(unit.realizations, self.realizations.len())?;
        check_span(unit.target_phones, sequence.phones.len())?;
        let oto = bank
            .oto
            .get(unit.oto.0 as usize)
            .ok_or(SpeechError::VoicebankInvalid)?;
        if unit.target_phones.is_empty() || unit.realizations.is_empty() {
            return Err(SpeechError::InvalidSpan);
        }
        if unit.start_us(&self.prosody) >= unit.end_us(&self.prosody)
            || unit.end_us(&self.prosody) > self.prosody.duration_us
        {
            return Err(SpeechError::VoicebankInvalid);
        }
        let atoms = &self.realizations[unit.realizations.range()];
        if unit.target_phones.start != atoms[0].target_phone
            || unit.target_phones.end != atoms[atoms.len() - 1].target_phone + 1
        {
            return Err(SpeechError::InvalidSpan);
        }
        let symbols = bank
            .alias_symbols(oto.alias)
            .ok_or(SpeechError::VoicebankInvalid)?;
        validate_symbols(atoms, symbols, unit.substitutions)?;
        Ok(())
    }
    fn validate_realizations(&self, sequence: &PhonemeSequence) -> Result<()> {
        let mut cursor = 0;
        for phone in 0..sequence.phones.len() {
            let start = cursor;
            while cursor < self.realizations.len()
                && self.realizations[cursor].target_phone as usize == phone
            {
                cursor += 1;
            }
            let symbols = &self.realizations[start..cursor];
            match symbols {
                [first] => {
                    if first.symbol
                        != crate::text::xsampa::bank_symbol(sequence.phones[phone].phone())
                    {
                        return Err(SpeechError::Pronunciation);
                    }
                }
                [first, second] => {
                    use crate::phone::ArpaPhone;
                    use BankSymbol::*;
                    let expected = match sequence.phones[phone].phone() {
                        ArpaPhone::Ch => (T, Sh),
                        ArpaPhone::Jh => (D, Zh),
                        ArpaPhone::Aw => (AShort, U),
                        ArpaPhone::Ay => (AShort, I),
                        ArpaPhone::Ey => (EShort, I),
                        ArpaPhone::Ow => (OShort, U),
                        ArpaPhone::Oy => (O, I),
                        _ => return Err(SpeechError::Pronunciation),
                    };
                    if (first.symbol, second.symbol) != expected {
                        return Err(SpeechError::Pronunciation);
                    }
                }
                _ => return Err(SpeechError::InvalidSpan),
            }
        }
        if cursor != self.realizations.len() {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
}

fn validate_symbols(
    atoms: &[PhoneSymbol],
    symbols: &[BankSymbol],
    changes: BankSubstitutions,
) -> Result<()> {
    let mut actual = symbols
        .iter()
        .flat_map(|symbol| symbol.parts().iter().copied())
        .filter(|symbol| *symbol != BankSymbol::Boundary);
    for (index, atom) in atoms.iter().enumerate() {
        let symbol = realized_symbol(atom.symbol, changes, index + 1 < atoms.len());
        for part in symbol.parts() {
            if actual.next() != Some(*part) {
                return Err(SpeechError::VoicebankInvalid);
            }
        }
    }
    if actual.next().is_some() {
        return Err(SpeechError::VoicebankInvalid);
    }
    Ok(())
}

fn realized_symbol(
    symbol: BankSymbol,
    changes: BankSubstitutions,
    before_consonant: bool,
) -> BankSymbol {
    use BankSubstitutions;
    match symbol {
        BankSymbol::Ah if changes.contains(BankSubstitutions::TETO_SCHWA) => BankSymbol::Schwa,
        BankSymbol::Ng if changes.contains(BankSubstitutions::VELAR_NASAL) => BankSymbol::N,
        BankSymbol::R if before_consonant && changes.contains(BankSubstitutions::RHOTIC) => {
            BankSymbol::Er
        }
        _ => symbol,
    }
}

#[cfg(all(test, feature = "cmudict"))]
mod tests {
    use super::*;
    use crate::{
        diagnostics::{BankSubstitutions, SpeechDiagnostics},
        input::SpeechInput,
        phone::ArpaPhone,
        text::{
            lexicon::Lexicon,
            phonemize::{CmudictPhonemizer, OovBackend},
        },
        voicebank::cvvc::TetoCvvcPlanner,
    };
    fn sequence(phones: &str) -> PhonemeSequence {
        let lexicon = Lexicon::parse(format!("test {phones}").as_bytes()).unwrap();
        let mut f =
            CmudictPhonemizer::new(lexicon, Lexicon::parse(b"").unwrap(), OovBackend::Disabled);
        let input = SpeechInput::new(SpeechKey::new(0, 1, 0).unwrap(), "test").unwrap();
        f.phonemize(&input, &mut SpeechDiagnostics::default())
            .unwrap()
    }
    fn plan(sequence: &PhonemeSequence, bank: &VoicebankIndex) -> Result<UnitPlan> {
        let timing = sequence
            .phones
            .iter()
            .enumerate()
            .map(|(i, _)| PhoneTiming {
                start_us: i as u64 * 100000,
                end_us: (i as u64 + 1) * 100000,
            })
            .collect();
        let prosody = ProsodyPlan::new(
            sequence,
            sequence.phones.len() as u64 * 100000,
            timing,
            vec![],
        )?;
        TetoCvvcPlanner::default().plan(sequence, prosody, bank, &mut SpeechDiagnostics::default())
    }
    #[test]
    fn planner_prefers_exact_alias_before_teto_substitution() {
        let sequence = sequence("B AH1");
        let bank = crate::testing::manifest(&["- bV", "- b@", "V -", "@ -"]).unwrap();
        let exact = plan(&sequence, &bank).unwrap();
        assert_eq!(
            exact.units[0]
                .alias(&bank)
                .and_then(|id| bank.alias_text(id)),
            Some("- bV")
        );
        assert_eq!(exact.units[0].substitutions, BankSubstitutions::empty());
        let bank = crate::testing::manifest(&["- b@", "@ -"]).unwrap();
        let fallback = plan(&sequence, &bank).unwrap();
        assert_eq!(
            fallback.units[0]
                .alias(&bank)
                .and_then(|id| bank.alias_text(id)),
            Some("- b@")
        );
        assert_eq!(
            fallback.units[0].substitutions,
            BankSubstitutions::TETO_SCHWA
        );
        assert_eq!(sequence.phones()[1].phone(), ArpaPhone::Ah);
    }
    #[test]
    fn planner_preserves_both_affricate_and_diphthong_components() {
        let sequence = sequence("CH AA1");
        let bank = crate::testing::manifest(&["- t", "t S", "SA", "A -"]).unwrap();
        let selected = plan(&sequence, &bank).unwrap();
        assert_eq!(selected.realizations[0].symbol, BankSymbol::T);
        assert_eq!(selected.realizations[1].symbol, BankSymbol::Sh);
        selected.validate(&sequence, &bank).unwrap();
        let combined = crate::testing::manifest(&["- tS", "SA", "A -"]).unwrap();
        plan(&sequence, &combined)
            .unwrap()
            .validate(&sequence, &combined)
            .unwrap();
        let sequence = super::tests::sequence("AW1");
        let bank = crate::testing::manifest(&["- a", "a U-"]).unwrap();
        let mut selected = plan(&sequence, &bank).unwrap();
        selected.validate(&sequence, &bank).unwrap();
        selected.units.pop();
        assert!(selected.validate(&sequence, &bank).is_err());
        selected.realizations.pop();
        assert!(selected.validate(&sequence, &bank).is_err());
    }
    #[test]
    fn planner_records_dark_l_and_schwa_together() {
        let sequence = sequence("AH1 L");
        let bank = crate::testing::manifest(&["- @", "@5"]).unwrap();
        let selected = plan(&sequence, &bank).unwrap();
        let unit = selected.units.last().unwrap();
        assert_eq!(
            unit.alias(&bank).and_then(|id| bank.alias_text(id)),
            Some("@5")
        );
        assert!(
            unit.substitutions
                .contains(BankSubstitutions::TETO_SCHWA | BankSubstitutions::DARK_L)
        );
    }
    #[test]
    fn planner_returns_missing_unit_without_partial_plan() {
        let sequence = sequence("B AA1");
        let bank = crate::testing::manifest(&["- A", "A -"]).unwrap();
        assert!(matches!(
            plan(&sequence, &bank),
            Err(SpeechError::MissingUnit { .. })
        ));
    }
    #[test]
    fn missing_vowel_transition_records_diagnostic() {
        let sequence = sequence("AA1 IY1");
        let bank = crate::testing::manifest(&["- A", "A -"]).unwrap();
        let timing = sequence
            .phones
            .iter()
            .enumerate()
            .map(|(i, _)| PhoneTiming {
                start_us: i as u64 * 100000,
                end_us: (i as u64 + 1) * 100000,
            })
            .collect();
        let prosody = ProsodyPlan::new(&sequence, 200000, timing, vec![]).unwrap();
        let mut diagnostics = SpeechDiagnostics::default();
        assert!(matches!(
            TetoCvvcPlanner::default().plan(&sequence, prosody, &bank, &mut diagnostics),
            Err(SpeechError::MissingUnit { .. })
        ));
        assert_eq!(diagnostics.counts()[5], 1);
    }
    #[test]
    fn symbol_validation_rejects_extra_alias_parts() {
        let mut symbols = Vec::new();
        assert!(BankSymbol::parse_alias("- bV", &mut symbols));
        let atoms = [PhoneSymbol {
            target_phone: 0,
            symbol: BankSymbol::B,
        }];
        assert!(validate_symbols(&atoms, &symbols, BankSubstitutions::empty()).is_err());
    }

    #[test]
    fn plan_rejects_wrong_revision_and_unit_provenance() {
        let sequence = sequence("AA1");
        let bank = crate::testing::manifest(&["- A", "A -"]).unwrap();
        let mut selected = plan(&sequence, &bank).unwrap();
        selected.sequence_revision = selected.sequence_revision.wrapping_add(1);
        assert!(selected.validate(&sequence, &bank).is_err());
        selected.sequence_revision = sequence.revision();
        selected.bank_revision[0] = 1;
        assert!(selected.validate(&sequence, &bank).is_err());
        selected.bank_revision = bank.revision;
        selected.units[0].oto = OtoRowId(u32::MAX);
        assert!(selected.validate(&sequence, &bank).is_err());
    }

    #[test]
    fn plan_rejects_wrong_unit_provenance() {
        let sequence = sequence("AA1");
        let bank = crate::testing::manifest(&["- A", "A -"]).unwrap();
        let mut selected = plan(&sequence, &bank).unwrap();
        selected.units[0].oto = OtoRowId(u32::MAX);
        assert!(selected.validate(&sequence, &bank).is_err());
    }
}
