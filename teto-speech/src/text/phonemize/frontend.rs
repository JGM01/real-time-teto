//! Lexical lookup with stable original text offsets.
use super::OovBackend;
use crate::{
    diagnostics::{Diagnostic, SpeechDiagnostics},
    error::{Result, SpeechError},
    text::{lexicon::Lexicon, syllable::SyllableSplitter},
    types::*,
};
use sha2::{Digest, Sha256};
pub struct CmudictPhonemizer {
    lexicon: Lexicon,
    overrides: Lexicon,
    oov: OovBackend,
    revision: [u8; 32],
    lookup: String,
}
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
        let tokens = tokenize(input.text())?;
        let mut phones = Vec::new();
        let mut words = Vec::with_capacity(tokens.len());
        let mut breaks = Vec::with_capacity(tokens.len());
        for (span, kind) in tokens {
            self.lookup.clear();
            for c in input.text()[span.range()].chars() {
                self.lookup.push(if c == '\u{2019}' {
                    '\''
                } else {
                    c.to_ascii_lowercase()
                });
            }
            if self.lookup.len() > MAX_WORD_ASCII {
                return Err(SpeechError::Bounds);
            }
            let start = phones.len();
            let (source, status) = self.pronounce(span, &mut phones, diagnostics)?;
            if phones.len() > MAX_PHONES || phones.len() == start {
                return Err(SpeechError::Bounds);
            }
            let end = phones.len() as u32;
            words.push(LexicalWord {
                text: span,
                phones: IndexSpan::new(start as u32, end)?,
                syllables: IndexSpan::new(0, 0)?,
                part_of_speech: PartOfSpeech::Unknown,
                source,
                status,
            });
            breaks.push(TextBreak {
                after_phone: end - 1,
                kind,
            });
        }
        let syllables = SyllableSplitter::split(&phones, &mut words, diagnostics)?;
        PhonemeSequence::new(
            input.clone(),
            phones,
            words,
            syllables,
            breaks,
            self.revision,
        )
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
                return Err(SpeechError::Bounds);
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
fn tokenize(text: &str) -> Result<Vec<(ByteSpan, BreakKind)>> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if c.is_ascii_alphabetic()
            || ((c == '\'' || c == '\u{2019}')
                && (start.is_some()
                    || text[index + c.len_utf8()..].starts_with(|n: char| n.is_ascii_alphabetic())))
        {
            start.get_or_insert(index);
            continue;
        }
        if let Some(first) = start.take() {
            if tokens.len() >= MAX_WORDS {
                return Err(SpeechError::Bounds);
            }
            tokens.push((ByteSpan::new(first as u32, index as u32)?, BreakKind::Word));
        }
        let kind = match c {
            ',' | ';' | ':' | '—' | '–' => BreakKind::MinorPhrase,
            '.' => BreakKind::Sentence,
            '?' => BreakKind::Question,
            '!' => BreakKind::Exclamation,
            ' ' | '-' | '"' | '(' | ')' | '\'' | '\u{2019}' | '“' | '”' => continue,
            _ if c.is_whitespace() => continue,
            _ => {
                return Err(SpeechError::UnsupportedText {
                    span: ByteSpan::new(index as u32, (index + c.len_utf8()) as u32)?,
                });
            }
        };
        if let Some((_, previous)) = tokens.last_mut() {
            *previous = kind;
        }
    }
    if tokens.is_empty() {
        return Err(SpeechError::Pronunciation);
    }
    Ok(tokens)
}
