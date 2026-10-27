//! English word lookup and bounded OOV prediction.
#[cfg(feature = "g2p-gru")]
pub use super::gru::G2pEnGru;

pub enum OovBackend {
    Disabled,
    #[cfg(feature = "g2p-gru")]
    Gru(Box<G2pEnGru>),
}

#[cfg(feature = "cmudict")]
use crate::{
    diagnostics::{Diagnostic, SpeechDiagnostics},
    error::{Result, SpeechError},
    input::SpeechInput,
    phone::{
        BreakKind, EnglishPhone, LexicalWord, PartOfSpeech, PronunciationSource,
        PronunciationStatus,
    },
    sequence::PhonemeSequence,
    span::{ByteSpan, IndexSpan},
    text::limits::{MAX_PHONES, MAX_WORD_ASCII, MAX_WORDS},
    text::{lexicon::Lexicon, syllable::syllabify},
};
#[cfg(feature = "cmudict")]
use sha2::{Digest, Sha256};
#[cfg(feature = "cmudict")]
pub struct CmudictPhonemizer {
    lexicon: Lexicon,
    overrides: Lexicon,
    oov: OovBackend,
    revision: [u8; 32],
    lookup: String,
}
#[cfg(feature = "cmudict")]
impl CmudictPhonemizer {
    pub fn new(lexicon: Lexicon, overrides: Lexicon, oov: OovBackend) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"teto-english-max-onset-v1");
        hash.update(lexicon.revision());
        hash.update(overrides.revision());
        match &oov {
            OovBackend::Disabled => hash.update(b"disabled"),
            #[cfg(feature = "g2p-gru")]
            OovBackend::Gru(gru) => hash.update(gru.revision()),
        }
        Self {
            lexicon,
            overrides,
            oov,
            revision: hash.finalize().into(),
            lookup: String::with_capacity(MAX_WORD_ASCII),
        }
    }
    pub fn embedded(oov: OovBackend) -> Result<Self> {
        Ok(Self::new(Lexicon::embedded()?, Lexicon::parse(b"")?, oov))
    }
    pub fn phonemize(
        &mut self,
        input: &SpeechInput,
        diagnostics: &mut SpeechDiagnostics,
    ) -> Result<PhonemeSequence> {
        let capacity = (input.text().len() / 4).clamp(1, MAX_WORDS);
        let mut phones = Vec::with_capacity(capacity.saturating_mul(4).min(MAX_PHONES));
        let mut words = Vec::with_capacity(capacity);
        let mut word_count = 0usize;
        tokenize(input.text(), |event| match event {
            TokenEvent::Word(span) => {
                self.lookup.clear();
                for character in input.text()[span.range()].chars() {
                    self.lookup.push(if character == '\u{2019}' {
                        '\''
                    } else {
                        character.to_ascii_lowercase()
                    });
                }
                if self.lookup.len() > MAX_WORD_ASCII {
                    return Err(SpeechError::LimitExceeded);
                }
                let start = phones.len();
                let (source, status) = self.pronounce(span, &mut phones, diagnostics)?;
                if phones.len() > MAX_PHONES || phones.len() == start {
                    return Err(SpeechError::LimitExceeded);
                }
                let end = phones.len() as u32;
                words.push(LexicalWord {
                    text: span,
                    phones: IndexSpan::new(start as u32, end)?,
                    syllables: IndexSpan::new(0, 0)?,
                    break_kind: BreakKind::Word,
                    part_of_speech: PartOfSpeech::Unknown,
                    source,
                    status,
                });
                word_count += 1;
                Ok(())
            }
            TokenEvent::Break(kind) => {
                if let Some(word) = words.last_mut() {
                    word.break_kind = kind;
                }
                Ok(())
            }
        })?;
        if word_count == 0 {
            return Err(SpeechError::Pronunciation);
        }
        let syllables = syllabify(&phones, &mut words, diagnostics)?;
        PhonemeSequence::new(input.clone(), phones, words, syllables, self.revision)
    }
    fn pronounce(
        &mut self,
        span: ByteSpan,
        phones: &mut Vec<EnglishPhone>,
        diagnostics: &mut SpeechDiagnostics,
    ) -> Result<(PronunciationSource, PronunciationStatus)> {
        let found = self
            .overrides
            .lookup(&self.lookup)
            .map(|p| (p, PronunciationSource::Override))
            .or_else(|| {
                self.lexicon
                    .lookup(&self.lookup)
                    .map(|p| (p, PronunciationSource::Lexicon))
            });
        if let Some(((pronunciation, alternatives), source)) = found {
            if phones.len() + pronunciation.len() > MAX_PHONES {
                return Err(SpeechError::LimitExceeded);
            }
            phones.extend_from_slice(pronunciation);
            let status = if alternatives > 1 {
                diagnostics.record(Diagnostic::Ambiguous {
                    word: span,
                    alternatives,
                });
                PronunciationStatus::Ambiguous
            } else {
                PronunciationStatus::Selected
            };
            return Ok((source, status));
        }
        diagnostics.record(Diagnostic::Oov { word: span });
        match &mut self.oov {
            OovBackend::Disabled => return Err(SpeechError::Unavailable),
            #[cfg(feature = "g2p-gru")]
            OovBackend::Gru(gru) => gru.predict_into(&self.lookup, phones)?,
        }
        #[allow(unreachable_code)]
        Ok((
            PronunciationSource::OovModel,
            PronunciationStatus::Estimated,
        ))
    }
}
#[cfg(feature = "cmudict")]
enum TokenEvent {
    Word(ByteSpan),
    Break(BreakKind),
}

#[cfg(feature = "cmudict")]
fn tokenize(text: &str, mut on_event: impl FnMut(TokenEvent) -> Result<()>) -> Result<()> {
    let mut start = None;
    let mut words = 0usize;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if character.is_ascii_alphabetic()
            || ((character == '\'' || character == '\u{2019}')
                && (start.is_some()
                    || text[index + character.len_utf8()..]
                        .starts_with(|next: char| next.is_ascii_alphabetic())))
        {
            start.get_or_insert(index);
            continue;
        }
        if let Some(first) = start.take() {
            words += 1;
            if words > MAX_WORDS {
                return Err(SpeechError::LimitExceeded);
            }
            on_event(TokenEvent::Word(ByteSpan::new(first as u32, index as u32)?))?;
        }
        let kind = match character {
            ',' | ';' | ':' | '—' | '–' => BreakKind::MinorPhrase,
            '.' => BreakKind::Sentence,
            '?' => BreakKind::Question,
            '!' => BreakKind::Exclamation,
            ' ' | '-' | '"' | '(' | ')' | '\'' | '\u{2019}' | '“' | '”' => continue,
            _ if character.is_whitespace() => continue,
            _ => {
                return Err(SpeechError::UnsupportedText {
                    span: ByteSpan::new(index as u32, (index + character.len_utf8()) as u32)?,
                });
            }
        };
        on_event(TokenEvent::Break(kind))?;
    }
    if words == 0 {
        return Err(SpeechError::Pronunciation);
    }
    Ok(())
}
