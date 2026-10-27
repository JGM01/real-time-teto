//! Checked target timing and complete selected-unit coverage.
use crate::{
    error::{Result, SpeechError},
    limits::*,
    sequence::check_span,
    types::*,
    voicebank::alias::BankSymbol,
};
#[derive(Debug)]
pub struct ProsodyPlan {
    key: SpeechKey,
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
        if timings.len() != sequence.phones.len() || contour.len() > MAX_REALIZATIONS {
            return Err(SpeechError::Bounds);
        }
        let mut end = 0;
        for (i, t) in timings.iter().enumerate() {
            if t.target_phone as usize != i
                || t.start_us < end
                || t.end_us <= t.start_us
                || t.end_us > duration_us
            {
                return Err(SpeechError::InvalidSpan);
            }
            end = t.end_us;
        }
        let mut previous = None;
        for p in &contour {
            if p.at_us > duration_us
                || p.f0_millihz == Some(0)
                || previous.is_some_and(|t| t >= p.at_us)
            {
                return Err(SpeechError::InvalidSpan);
            }
            previous = Some(p.at_us);
        }
        Ok(Self {
            key: sequence.input.key(),
            duration_us,
            timings,
            contour,
        })
    }
    pub fn key(&self) -> SpeechKey {
        self.key
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
pub enum RealizationPart {
    Whole,
    First,
    Second,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Realization {
    pub target_phone: u32,
    pub symbol: BankSymbol,
    pub part: RealizationPart,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedUnit {
    pub unit: UnitId,
    pub alias: AliasId,
    pub target_phones: IndexSpan,
    pub realizations: IndexSpan,
    pub start_us: u64,
    pub end_us: u64,
    pub pitch: PitchId,
    pub substitutions: crate::diagnostics::Substitutions,
}
#[derive(Debug)]
pub struct UnitPlan {
    pub(crate) key: SpeechKey,
    pub(crate) bank_revision: [u8; 32],
    pub(crate) units: Vec<PlannedUnit>,
    pub(crate) groups: Vec<IndexSpan>,
    pub(crate) realizations: Vec<Realization>,
    pub(crate) prosody: ProsodyPlan,
}
impl UnitPlan {
    pub fn key(&self) -> SpeechKey {
        self.key
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
    pub fn realizations(&self) -> &[Realization] {
        &self.realizations
    }
    pub fn prosody(&self) -> &ProsodyPlan {
        &self.prosody
    }
    pub fn validate(&self, sequence: &PhonemeSequence, bank: &VoicebankManifest) -> Result<()> {
        if self.key != sequence.input.key()
            || self.prosody.key != self.key
            || self.bank_revision != bank.revision
            || self.prosody.timings.len() != sequence.phones.len()
        {
            return Err(SpeechError::InvalidInput);
        }
        if self.units.is_empty()
            || self.units.len() > MAX_UNITS
            || self.realizations.len() > MAX_REALIZATIONS
        {
            return Err(SpeechError::Bounds);
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
                .contains(crate::diagnostics::Substitutions::COVERAGE_REPAIR)
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
        bank: &VoicebankManifest,
    ) -> Result<()> {
        check_span(unit.realizations, self.realizations.len())?;
        check_span(unit.target_phones, sequence.phones.len())?;
        let oto = bank
            .oto
            .get(unit.unit.0 as usize)
            .ok_or(SpeechError::VoicebankInvalid)?;
        if unit.alias != oto.alias
            || unit.pitch != oto.pitch
            || unit.start_us >= unit.end_us
            || unit.end_us > self.prosody.duration_us
            || unit.realizations.is_empty()
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
            .alias_symbols(unit.alias)
            .ok_or(SpeechError::VoicebankInvalid)?;
        validate_symbols(atoms, symbols, unit.substitutions)?;
        if unit.start_us != self.prosody.timings[unit.target_phones.start as usize].start_us
            || unit.end_us != self.prosody.timings[unit.target_phones.end as usize - 1].end_us
        {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
    fn validate_realizations(&self, sequence: &PhonemeSequence) -> Result<()> {
        let mut cursor = 0;
        for phone in 0..sequence.phones.len() {
            let first = self
                .realizations
                .get(cursor)
                .ok_or(SpeechError::InvalidSpan)?;
            if first.target_phone as usize != phone {
                return Err(SpeechError::InvalidSpan);
            }
            match first.part {
                RealizationPart::Whole => {
                    if first.symbol
                        != crate::text::xsampa::TetoXsampaMap::map(sequence.phones[phone].phone())
                    {
                        return Err(SpeechError::Pronunciation);
                    }
                    cursor += 1;
                }
                RealizationPart::First => {
                    let second = self
                        .realizations
                        .get(cursor + 1)
                        .ok_or(SpeechError::InvalidSpan)?;
                    if second.part != RealizationPart::Second
                        || second.target_phone != first.target_phone
                    {
                        return Err(SpeechError::InvalidSpan);
                    }
                    use crate::types::ArpaPhone;
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
                    cursor += 2;
                }
                RealizationPart::Second => return Err(SpeechError::InvalidSpan),
            }
        }
        if cursor != self.realizations.len() {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
}

fn validate_symbols(
    atoms: &[Realization],
    symbols: &[BankSymbol],
    changes: crate::diagnostics::Substitutions,
) -> Result<()> {
    let mut normalized = arrayvec::ArrayVec::<BankSymbol, { MAX_ALIAS_BYTES * 2 }>::new();
    for symbol in symbols {
        for part in symbol.parts() {
            normalized
                .try_push(*part)
                .map_err(|_| SpeechError::Bounds)?;
        }
    }
    let mut cursor = 0;
    for (index, atom) in atoms.iter().enumerate() {
        let symbol = realized_symbol(atom.symbol, changes, index + 1 < atoms.len());
        for part in symbol.parts() {
            let offset = normalized[cursor..]
                .iter()
                .position(|s| s == part)
                .ok_or(SpeechError::VoicebankInvalid)?;
            cursor += offset + 1;
        }
    }
    Ok(())
}

fn realized_symbol(
    symbol: BankSymbol,
    changes: crate::diagnostics::Substitutions,
    before_consonant: bool,
) -> BankSymbol {
    use crate::diagnostics::Substitutions;
    match symbol {
        BankSymbol::Ah if changes.contains(Substitutions::TETO_SCHWA) => BankSymbol::Schwa,
        BankSymbol::Ng if changes.contains(Substitutions::VELAR_NASAL) => BankSymbol::N,
        BankSymbol::R if before_consonant && changes.contains(Substitutions::RHOTIC) => {
            BankSymbol::Er
        }
        _ => symbol,
    }
}

#[cfg(all(test, feature = "cmudict"))]
mod tests {
    use super::*;
    use crate::{
        diagnostics::{SpeechDiagnostics, Substitutions},
        text::{
            lexicon::Lexicon,
            phonemize::{CmudictPhonemizer, OovBackend},
        },
        voicebank::select::TetoCvvcPlanner,
    };
    fn sequence(phones: &str) -> PhonemeSequence {
        let lexicon = Lexicon::parse(format!("test {phones}").as_bytes()).unwrap();
        let mut f =
            CmudictPhonemizer::new(lexicon, Lexicon::parse(b"").unwrap(), OovBackend::Disabled);
        let input = SpeechInput::new(SpeechKey::new(0, 1, 0).unwrap(), "test").unwrap();
        f.phonemize(&input, &mut SpeechDiagnostics::default())
            .unwrap()
    }
    fn plan(sequence: &PhonemeSequence, bank: &VoicebankManifest) -> Result<UnitPlan> {
        let timing = sequence
            .phones
            .iter()
            .enumerate()
            .map(|(i, _)| PhoneTiming {
                target_phone: i as u32,
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
        assert_eq!(bank.alias_text(exact.units[0].alias), Some("- bV"));
        assert_eq!(exact.units[0].substitutions, Substitutions::empty());
        let bank = crate::testing::manifest(&["- b@", "@ -"]).unwrap();
        let fallback = plan(&sequence, &bank).unwrap();
        assert_eq!(bank.alias_text(fallback.units[0].alias), Some("- b@"));
        assert_eq!(fallback.units[0].substitutions, Substitutions::TETO_SCHWA);
        assert_eq!(sequence.phones()[1].phone(), ArpaPhone::Ah);
    }
    #[test]
    fn planner_preserves_both_affricate_and_diphthong_components() {
        let sequence = sequence("CH AA1");
        let bank = crate::testing::manifest(&["- t", "t S", "SA", "A -"]).unwrap();
        let selected = plan(&sequence, &bank).unwrap();
        assert_eq!(selected.realizations[0].part, RealizationPart::First);
        assert_eq!(selected.realizations[1].part, RealizationPart::Second);
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
        assert!(selected.units.iter().any(|u| {
            u.substitutions
                .contains(Substitutions::TETO_SCHWA | Substitutions::DARK_L)
        }));
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
                target_phone: i as u32,
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
    fn plan_rejects_wrong_revision_and_unit_provenance() {
        let sequence = sequence("AA1");
        let bank = crate::testing::manifest(&["- A", "A -"]).unwrap();
        let mut selected = plan(&sequence, &bank).unwrap();
        selected.bank_revision[0] = 1;
        assert!(selected.validate(&sequence, &bank).is_err());
        selected.bank_revision = bank.revision;
        selected.units[0].unit = UnitId(u32::MAX);
        assert!(selected.validate(&sequence, &bank).is_err());
    }
}
