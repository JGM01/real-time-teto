//! Deterministic contextual CVVC selection.
type AliasText = arrayvec::ArrayString<MAX_ALIAS_BYTES>;
macro_rules! alias {($($args:tt)*)=>{crate::voicebank::select::alias_text(format_args!($($args)*))?};}
macro_rules! list {($($args:expr),* $(,)?)=>{arrayvec::ArrayVec::<AliasText,5>::from_iter([$($args),*])};}
mod realize;
mod syllable;
use super::alias::BankSymbol;
use crate::{
    diagnostics::{Diagnostic, SpeechDiagnostics, Substitution, Substitutions},
    error::{Result, SpeechError},
    limits::*,
    text::xsampa::TetoXsampaMap,
    types::*,
};
#[derive(Debug, Clone, Copy)]
pub enum PitchPolicy {
    Exact(PitchId),
    Prefer(PitchId),
}
pub struct TetoCvvcPlanner {
    pub pitch: PitchPolicy,
}
impl Default for TetoCvvcPlanner {
    fn default() -> Self {
        Self {
            pitch: PitchPolicy::Exact(PitchId(0)),
        }
    }
}
struct Selection<'a> {
    bank: &'a VoicebankManifest,
    pitch: PitchPolicy,
    probes: usize,
    units: Vec<PlannedUnit>,
    groups: Vec<IndexSpan>,
    atoms: Vec<Realization>,
    phone_atoms: Vec<IndexSpan>,
    diagnostics: &'a mut SpeechDiagnostics,
    prosody: &'a ProsodyPlan,
}
impl TetoCvvcPlanner {
    pub fn plan(
        &mut self,
        sequence: &PhonemeSequence,
        prosody: ProsodyPlan,
        bank: &VoicebankManifest,
        diagnostics: &mut SpeechDiagnostics,
    ) -> Result<UnitPlan> {
        if prosody.key() != sequence.input.key() || prosody.timings().len() != sequence.phones.len()
        {
            return Err(SpeechError::InvalidInput);
        }
        let pitch = match self.pitch {
            PitchPolicy::Exact(p) | PitchPolicy::Prefer(p) => p,
        };
        if pitch.0 as usize >= bank.pitches.len() {
            return Err(SpeechError::VoicebankInvalid);
        }
        let (atoms, phone_atoms) = realize::components(sequence, bank, pitch, diagnostics)?;
        let mut selection = Selection {
            bank,
            pitch: self.pitch,
            probes: 0,
            units: Vec::new(),
            groups: Vec::new(),
            atoms,
            phone_atoms,
            diagnostics,
            prosody: &prosody,
        };
        selection.select(sequence)?;
        let units = selection.units;
        let realizations = selection.atoms;
        let groups = selection.groups;
        let plan = UnitPlan {
            key: sequence.input.key(),
            bank_revision: bank.revision,
            units,
            groups,
            realizations,
            prosody,
        };
        plan.validate(sequence, bank)?;
        Ok(plan)
    }
}
impl Selection<'_> {
    fn select(&mut self, sequence: &PhonemeSequence) -> Result<()> {
        let mut previous = None;
        for syllable in &sequence.syllables {
            let nucleus = self.phone_atoms[syllable.nucleus.start as usize].start as usize;
            let start =
                previous.unwrap_or(self.phone_atoms[syllable.phones.start as usize].start as usize);
            let word_start = sequence
                .words
                .iter()
                .find(|w| {
                    w.phones.start <= syllable.nucleus.start
                        && w.phones.end > syllable.nucleus.start
                })
                .ok_or(SpeechError::InvalidSpan)?
                .phones
                .start as usize;
            let group_start = self.units.len();
            self.port_syllable(
                start,
                nucleus,
                previous.is_none(),
                self.phone_atoms[word_start].start as usize,
            )?;
            self.groups
                .push(IndexSpan::new(group_start as u32, self.units.len() as u32)?);
            let next_break = sequence.breaks.iter().any(|b| {
                b.after_phone >= syllable.nucleus.start
                    && b.after_phone < syllable.phones.end
                    && b.kind != BreakKind::Word
            });
            if next_break {
                let group_start = self.units.len();
                self.ending(
                    nucleus,
                    self.phone_atoms[syllable.phones.end as usize - 1].end as usize,
                )?;
                self.groups
                    .push(IndexSpan::new(group_start as u32, self.units.len() as u32)?);
                previous = None;
            } else {
                previous = Some(nucleus);
            }
        }
        if let Some(nucleus) = previous {
            let group_start = self.units.len();
            self.ending(nucleus, self.atoms.len())?;
            self.groups
                .push(IndexSpan::new(group_start as u32, self.units.len() as u32)?);
        }
        Ok(())
    }
    fn vv(&mut self, start: usize, nucleus: usize) -> Result<()> {
        let a = self.atoms[start].symbol;
        let b = self.atoms[nucleus].symbol.text();
        if self.choose(
            &[alias!("{} {b}", a.text()), alias!("{}{b}", a.text())],
            start,
            nucleus + 1,
        )? {
            return Ok(());
        }
        let glide = match a {
            BankSymbol::Ay | BankSymbol::Ey | BankSymbol::Oy => Some("j"),
            BankSymbol::Aw | BankSymbol::Ow => Some("w"),
            _ => None,
        };
        if let Some(glide) = glide {
            let _ = self.choose(
                &[
                    alias!("{} {glide}", a.text()),
                    alias!("{}{glide}", a.text()),
                ],
                start,
                start + 1,
            )?;
            if self.choose(
                &[alias!("{glide}{b}"), alias!("{glide} {b}")],
                nucleus,
                nucleus + 1,
            )? {
                self.diagnostics.record(Diagnostic::Substitution {
                    phones: IndexSpan::new(start as u32, nucleus as u32 + 1)?,
                    reason: Substitution::Glide,
                });
                return Ok(());
            }
        }
        if self.choose(&[alias!("{b}"), alias!("- {b}")], nucleus, nucleus + 1)? {
            return Ok(());
        }
        self.missing(nucleus, nucleus + 1)
    }
    fn ending(&mut self, start: usize, end: usize) -> Result<()> {
        if end == start + 1 {
            let a = self.atoms[start].symbol.text();
            if self.choose(&[alias!("{a} -"), alias!("{a}-")], start, end)? {
                return Ok(());
            }
            return Ok(());
        }
        self.ending_clusters(start, end)
    }
    fn ending_clusters(&mut self, start: usize, end: usize) -> Result<()> {
        let mut cursor = start;
        while cursor < end {
            let last = cursor + 1 == end;
            if last
                && self.units.iter().any(|u| {
                    u.realizations.start as usize <= cursor && u.realizations.end as usize > cursor
                })
            {
                break;
            }
            let mut selected = false;
            for stop in (cursor + 2..=end.min(cursor + 4)).rev() {
                let a = self.atoms[cursor].symbol.text();
                let tail = self.join(cursor + 1, stop, "")?;
                let mut candidates = arrayvec::ArrayVec::<AliasText, 8>::new();
                if stop == end {
                    candidates.push(alias!("{a} {tail}-"));
                }
                candidates.push(alias!("{a} {tail}"));
                candidates.push(alias!("{a}{tail}"));
                let spaced = self.join(cursor + 1, stop, " ")?;
                if stop == end {
                    candidates.push(alias!("{a}{spaced}-"));
                }
                candidates.push(alias!("{a} {spaced}"));
                candidates.push(alias!("{a}{spaced}"));
                if self.choose(&candidates, cursor, stop)? {
                    cursor = stop - 1;
                    selected = true;
                    break;
                }
            }
            if selected {
                continue;
            }
            let a = self.atoms[cursor].symbol.text();
            let candidates = list![alias!("{a}"), alias!("{a}-"), alias!("{a} -")];
            if !self.choose(&candidates, cursor, cursor + 1)? {
                return self.missing(cursor, cursor + 1);
            }
            cursor += 1;
        }
        Ok(())
    }
    fn join(&self, start: usize, end: usize, separator: &str) -> Result<AliasText> {
        let mut output = AliasText::new();
        for (index, atom) in self.atoms[start..end].iter().enumerate() {
            if index > 0 {
                output
                    .try_push_str(separator)
                    .map_err(|_| SpeechError::Bounds)?;
            }
            output
                .try_push_str(atom.symbol.text())
                .map_err(|_| SpeechError::Bounds)?;
        }
        Ok(output)
    }
    fn choose(&mut self, candidates: &[AliasText], start: usize, end: usize) -> Result<bool> {
        if let Some((id, row, reason)) = self.find_candidates(candidates)? {
            self.add(id, row, start, end, reason)?;
            return Ok(true);
        }
        Ok(false)
    }
    fn find_candidates(
        &mut self,
        candidates: &[AliasText],
    ) -> Result<Option<(UnitId, OtoEntry, Substitutions)>> {
        let requested = match self.pitch {
            PitchPolicy::Exact(p) | PitchPolicy::Prefer(p) => p,
        };
        for candidate in candidates {
            if let Some(result) = self.lookup(candidate, requested)? {
                return Ok(Some((result.0, result.1, Substitutions::empty())));
            }
            let (fallback, changes) = fallback_alias(candidate, self.bank, requested)?;
            if !changes.is_empty()
                && let Some(result) = self.lookup(&fallback, requested)?
            {
                return Ok(Some((result.0, result.1, changes)));
            }
        }
        if let PitchPolicy::Prefer(_) = self.pitch {
            for p in 0..self.bank.pitches.len() {
                if p as u32 == requested.0 {
                    continue;
                }
                for candidate in candidates {
                    if let Some(result) = self.lookup(candidate, PitchId(p as u32))? {
                        self.diagnostics.record(Diagnostic::PitchFallback {
                            requested,
                            selected: PitchId(p as u32),
                        });
                        return Ok(Some((result.0, result.1, Substitutions::empty())));
                    }
                }
            }
        }
        Ok(None)
    }
    fn lookup(&mut self, text: &str, pitch: PitchId) -> Result<Option<(UnitId, OtoEntry)>> {
        if self.probes >= MAX_CANDIDATE_PROBES {
            return Err(SpeechError::Bounds);
        }
        self.probes += 1;
        Ok(self.bank.find(text, pitch).map(|(id, row)| (id, *row)))
    }
    fn add(
        &mut self,
        id: UnitId,
        row: OtoEntry,
        start: usize,
        end: usize,
        reason: Substitutions,
    ) -> Result<()> {
        if self.units.len() >= MAX_UNITS {
            return Err(SpeechError::Bounds);
        }
        for reason in reason.reasons() {
            self.diagnostics.record(Diagnostic::Substitution {
                phones: IndexSpan::new(
                    self.atoms[start].target_phone,
                    self.atoms[end - 1].target_phone + 1,
                )?,
                reason,
            });
        }
        let first = self.atoms[start].target_phone;
        let last = self.atoms[end - 1].target_phone;
        self.units.push(PlannedUnit {
            unit: id,
            alias: row.alias,
            target_phones: IndexSpan::new(first, last + 1)?,
            realizations: IndexSpan::new(start as u32, end as u32)?,
            start_us: self.prosody.timings()[first as usize].start_us,
            end_us: self.prosody.timings()[last as usize].end_us,
            pitch: row.pitch,
            substitutions: reason,
        });
        Ok(())
    }
    fn error(&self, start: usize, end: usize) -> SpeechError {
        SpeechError::MissingUnit {
            phones: IndexSpan {
                start: self.atoms[start].target_phone,
                end: self.atoms[end - 1].target_phone + 1,
            },
            pitch: match self.pitch {
                PitchPolicy::Exact(p) | PitchPolicy::Prefer(p) => p,
            },
            probes: self.probes as u32,
        }
    }
    fn missing(&mut self, start: usize, end: usize) -> Result<()> {
        let error = self.error(start, end);
        if let SpeechError::MissingUnit { phones, pitch, .. } = error {
            self.diagnostics
                .record(Diagnostic::MissingAlias { phones, pitch });
        }
        Err(error)
    }
}

fn alias_text(arguments: std::fmt::Arguments<'_>) -> Result<AliasText> {
    use std::fmt::Write;
    let mut output = AliasText::new();
    output
        .write_fmt(arguments)
        .map_err(|_| SpeechError::Bounds)?;
    Ok(output)
}
fn fallback_alias(
    alias: &str,
    bank: &VoicebankManifest,
    pitch: PitchId,
) -> Result<(AliasText, Substitutions)> {
    let schwa = bank.find("- bV", pitch).is_none() && bank.find("bV", pitch).is_none();
    let nasal = ["N g", "N g-", "N k", "N k-"]
        .iter()
        .any(|s| bank.find(s, pitch).is_none());
    let dark_l = ["@5", "u5", "O5", "A5", "E5", "I5", "i5"]
        .iter()
        .any(|a| bank.find(a, pitch).is_some());
    let mut text = AliasText::new();
    let mut changes = Substitutions::empty();
    let mut remaining = alias;
    while !remaining.is_empty() {
        let first = remaining.as_bytes()[0];
        let replacement = if first == b'V' && schwa {
            changes |= Substitutions::TETO_SCHWA;
            '@'
        } else if nasal && (remaining.starts_with("N g") || remaining.starts_with("N k")) {
            changes |= Substitutions::VELAR_NASAL;
            'n'
        } else if remaining.starts_with("r ")
            && remaining
                .as_bytes()
                .get(2)
                .is_some_and(|b| b"btDd fghk lmnNpr sStTvwjzZ".contains(b) && *b != b' ')
        {
            changes |= Substitutions::RHOTIC;
            '3'
        } else if first == b'l'
            && dark_l
            && text
                .as_bytes()
                .last()
                .is_some_and(|b| b"@uOAEIi".contains(b))
        {
            changes |= Substitutions::DARK_L;
            '5'
        } else {
            first as char
        };
        text.try_push(replacement)
            .map_err(|_| SpeechError::Bounds)?;
        remaining = &remaining[1..];
    }
    Ok((text, changes))
}
