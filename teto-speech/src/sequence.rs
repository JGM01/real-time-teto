//! Checked lexical and syllable partitions.
use crate::{
    error::{Result, SpeechError},
    types::*,
};

#[derive(Debug)]
pub struct PhonemeSequence {
    pub(crate) input: SpeechInput,
    pub(crate) phones: Vec<EnglishPhone>,
    pub(crate) words: Vec<LexicalWord>,
    pub(crate) syllables: Vec<Syllable>,
    pub(crate) breaks: Vec<TextBreak>,
    pub(crate) frontend_revision: [u8; 32],
}
impl PhonemeSequence {
    pub fn new(
        input: SpeechInput,
        phones: Vec<EnglishPhone>,
        words: Vec<LexicalWord>,
        syllables: Vec<Syllable>,
        breaks: Vec<TextBreak>,
        frontend_revision: [u8; 32],
    ) -> Result<Self> {
        let value = Self {
            input,
            phones,
            words,
            syllables,
            breaks,
            frontend_revision,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn input(&self) -> &SpeechInput {
        &self.input
    }
    pub fn phones(&self) -> &[EnglishPhone] {
        &self.phones
    }
    pub fn words(&self) -> &[LexicalWord] {
        &self.words
    }
    pub fn syllables(&self) -> &[Syllable] {
        &self.syllables
    }
    pub fn breaks(&self) -> &[TextBreak] {
        &self.breaks
    }
    pub fn frontend_revision(&self) -> [u8; 32] {
        self.frontend_revision
    }

    pub fn validate(&self) -> Result<()> {
        if self.phones.is_empty() || self.words.is_empty() || self.syllables.is_empty() {
            return Err(SpeechError::Pronunciation);
        }
        if self.phones.len() > MAX_PHONES
            || self.words.len() > MAX_WORDS
            || self.syllables.len() > MAX_SYLLABLES
            || self.breaks.len() > MAX_BREAKS
        {
            return Err(SpeechError::Bounds);
        }
        let (mut phone, mut syllable, mut byte) = (0, 0, 0);
        for word in &self.words {
            check_span(word.phones, self.phones.len())?;
            check_span(word.syllables, self.syllables.len())?;
            if word.text.start < byte
                || word.text.start >= word.text.end
                || self.input.text().get(word.text.range()).is_none()
                || word.phones.start != phone
                || word.syllables.start != syllable
                || word.phones.is_empty()
                || word.syllables.is_empty()
            {
                return Err(SpeechError::InvalidSpan);
            }
            self.validate_word(word)?;
            (phone, syllable, byte) = (word.phones.end, word.syllables.end, word.text.end);
        }
        if phone as usize != self.phones.len() || syllable as usize != self.syllables.len() {
            return Err(SpeechError::InvalidSpan);
        }
        let mut previous = None;
        for item in &self.breaks {
            if item.after_phone as usize >= self.phones.len()
                || previous.is_some_and(|p| p >= item.after_phone)
            {
                return Err(SpeechError::InvalidSpan);
            }
            previous = Some(item.after_phone);
        }
        Ok(())
    }
    fn validate_word(&self, word: &LexicalWord) -> Result<()> {
        let mut cursor = word.phones.start;
        for s in &self.syllables[word.syllables.range()] {
            for span in [s.phones, s.onset, s.nucleus, s.coda] {
                check_span(span, self.phones.len())?;
            }
            if s.phones.start != cursor
                || s.phones.end > word.phones.end
                || s.onset.start != cursor
                || s.onset.end != s.nucleus.start
                || s.nucleus.len() != 1
                || s.nucleus.end != s.coda.start
                || s.coda.end != s.phones.end
            {
                return Err(SpeechError::InvalidSpan);
            }
            let nucleus = self.phones[s.nucleus.start as usize];
            if !nucleus.phone.is_vowel()
                || nucleus.stress != Some(s.stress)
                || self.phones[s.onset.range()]
                    .iter()
                    .chain(&self.phones[s.coda.range()])
                    .any(|p| p.phone.is_vowel())
            {
                return Err(SpeechError::Pronunciation);
            }
            cursor = s.phones.end;
        }
        if cursor != word.phones.end {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
}
pub(crate) fn check_span(span: IndexSpan, len: usize) -> Result<()> {
    if span.start > span.end || span.end as usize > len {
        return Err(SpeechError::InvalidSpan);
    }
    Ok(())
}
