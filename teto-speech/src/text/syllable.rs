//! Deterministic maximum-onset English syllabification.
use crate::{
    diagnostics::{Diagnostic, SpeechDiagnostics},
    error::{Result, SpeechError},
    phone::{ArpaPhone, EnglishPhone, LexicalWord, Stress, Syllable},
    span::IndexSpan,
    text::limits::{MAX_SYLLABLES, MAX_WORD_PHONES},
};

pub(crate) fn syllabify(
    phones: &[EnglishPhone],
    words: &mut [LexicalWord],
    diagnostics: &mut SpeechDiagnostics,
) -> Result<Vec<Syllable>> {
    let mut syllables = Vec::new();
    for word in words {
        let first = syllables.len();
        split_word(phones, word.phones, &mut syllables, diagnostics)?;
        word.syllables = IndexSpan::new(first as u32, syllables.len() as u32)?;
    }
    Ok(syllables)
}
fn split_word(
    phones: &[EnglishPhone],
    span: IndexSpan,
    output: &mut Vec<Syllable>,
    diagnostics: &mut SpeechDiagnostics,
) -> Result<()> {
    let mut nuclei = [0u32; MAX_WORD_PHONES];
    let mut count = 0;
    for i in span.start..span.end {
        if phones[i as usize].phone.is_vowel() {
            if count == nuclei.len() {
                return Err(SpeechError::LimitExceeded);
            }
            nuclei[count] = i;
            count += 1;
        }
    }
    if count == 0 {
        return Err(SpeechError::Pronunciation);
    }
    let mut start = span.start;
    for (i, nucleus) in nuclei[..count].iter().copied().enumerate() {
        if output.len() >= MAX_SYLLABLES {
            return Err(SpeechError::LimitExceeded);
        }
        let end = if i + 1 < count {
            boundary(phones, nucleus + 1, nuclei[i + 1], diagnostics)?
        } else {
            span.end
        };
        output.push(Syllable {
            phones: IndexSpan::new(start, end)?,
            nucleus,
            stress: phones[nucleus as usize].stress.unwrap_or(Stress::Unknown),
        });
        start = end;
    }
    Ok(())
}
fn boundary(
    phones: &[EnglishPhone],
    start: u32,
    end: u32,
    diagnostics: &mut SpeechDiagnostics,
) -> Result<u32> {
    if start == end {
        return Ok(end);
    }
    for candidate in start..end {
        if legal_onset(&phones[candidate as usize..end as usize]) {
            if candidate > start && end - start > 1 {
                diagnostics.record(Diagnostic::UncertainSyllable {
                    phones: IndexSpan::new(start, end)?,
                });
            }
            return Ok(candidate);
        }
    }
    Ok(end)
}
fn legal_onset(phones: &[EnglishPhone]) -> bool {
    use ArpaPhone::*;
    match phones {
        [a] => a.phone != Ng,
        [a, b] => matches!(
            (a.phone, b.phone),
            (P | B | T | D | K | G | F | Th | Sh, R)
                | (P | B | K | G | F | S, L)
                | (S, P | T | K | F | M | N | W)
                | (T | D | K | G | Hh | Th, W)
                | (P | B | T | D | K | G | F | V | Hh | M | N | L | S | Z, Y)
        ),
        [a, b, c] => {
            a.phone == S
                && matches!(
                    (b.phone, c.phone),
                    (P | T | K, R) | (P | K, L) | (K, W) | (P | T | K | M | N, Y)
                )
        }
        _ => false,
    }
}
