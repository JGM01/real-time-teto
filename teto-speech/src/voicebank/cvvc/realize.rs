//! Explicit phone components for missing combined recordings.
use super::*;
use crate::plan::MAX_REALIZATIONS;
pub(super) fn components(
    sequence: &PhonemeSequence,
    bank: &VoicebankIndex,
    pitch: PitchId,
    diagnostics: &mut SpeechDiagnostics,
) -> Result<(Vec<PhoneSymbol>, Vec<IndexSpan>)> {
    let mut atoms = Vec::with_capacity(sequence.phones.len());
    let mut spans = Vec::with_capacity(sequence.phones.len());
    for (index, phone) in sequence.phones.iter().enumerate() {
        let symbol = bank_symbol(phone.phone());
        let start = atoms.len();
        let split = split_for_bank(symbol, bank, pitch)?;
        if let Some((a, b, reason)) = split {
            atoms.push(PhoneSymbol {
                target_phone: index as u32,
                symbol: a,
            });
            atoms.push(PhoneSymbol {
                target_phone: index as u32,
                symbol: b,
            });
            diagnostics.record(Diagnostic::Substitution {
                phones: IndexSpan::new(index as u32, index as u32 + 1)?,
                reason,
            });
        } else {
            atoms.push(PhoneSymbol {
                target_phone: index as u32,
                symbol,
            });
        }
        spans.push(IndexSpan::new(start as u32, atoms.len() as u32)?);
    }
    if atoms.len() > MAX_REALIZATIONS {
        return Err(SpeechError::LimitExceeded);
    }
    Ok((atoms, spans))
}
fn split_for_bank(
    symbol: BankSymbol,
    bank: &VoicebankIndex,
    pitch: PitchId,
) -> Result<Option<(BankSymbol, BankSymbol, Substitution)>> {
    use BankSymbol::*;
    let (a, b, reason) = match symbol {
        Ch => (T, Sh, Substitution::AffricateSplit),
        Jh => (D, Zh, Substitution::AffricateSplit),
        Ay => (AShort, I, Substitution::DiphthongSplit),
        Aw => (AShort, U, Substitution::DiphthongSplit),
        Ey => (EShort, I, Substitution::DiphthongSplit),
        Ow => (OShort, U, Substitution::DiphthongSplit),
        Oy => (O, I, Substitution::DiphthongSplit),
        _ => return Ok(None),
    };
    let text = symbol.text();
    let combined = if matches!(symbol, Ch | Jh) {
        [
            alias!("{text}A"),
            alias!("{text} A"),
            alias!("{text}Q"),
            alias!("{text} Q"),
        ]
        .iter()
        .any(|a| bank.find(a, pitch).is_some())
    } else {
        bank.find(text, pitch).is_some() || bank.find(&alias!("- {text}"), pitch).is_some()
    };
    Ok(if combined { None } else { Some((a, b, reason)) })
}
