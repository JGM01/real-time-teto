//! Checked lexical and syllable partitions.
use crate::{
    error::{Result, SpeechError},
    input::SpeechInput,
    phone::{EnglishPhone, LexicalWord, Syllable, TextBreak},
    span::check_span,
    text::limits::{MAX_PHONES, MAX_SYLLABLES, MAX_WORDS},
};

#[derive(Debug)]
pub struct PhonemeSequence {
    pub(crate) input: SpeechInput,
    pub(crate) phones: Vec<EnglishPhone>,
    pub(crate) words: Vec<LexicalWord>,
    pub(crate) syllables: Vec<Syllable>,
    pub(crate) frontend_revision: [u8; 32],
    pub(crate) revision: u64,
}
impl PhonemeSequence {
    pub fn new(
        input: SpeechInput,
        phones: Vec<EnglishPhone>,
        words: Vec<LexicalWord>,
        syllables: Vec<Syllable>,
        frontend_revision: [u8; 32],
    ) -> Result<Self> {
        let mut revision = input.revision();
        for byte in frontend_revision {
            revision = crate::input::hash_byte(revision, u32::from(byte));
        }
        let value = Self {
            input,
            phones,
            words,
            syllables,
            frontend_revision,
            revision,
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
    pub fn breaks(&self) -> impl Iterator<Item = TextBreak> + '_ {
        self.words.iter().map(|word| TextBreak {
            after_phone: word.phones.end - 1,
            kind: word.break_kind,
        })
    }
    pub fn frontend_revision(&self) -> [u8; 32] {
        self.frontend_revision
    }

    /// Return the exact sequence revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn validate(&self) -> Result<()> {
        if self.phones.is_empty() || self.words.is_empty() || self.syllables.is_empty() {
            return Err(SpeechError::Pronunciation);
        }
        if self.phones.len() > MAX_PHONES
            || self.words.len() > MAX_WORDS
            || self.syllables.len() > MAX_SYLLABLES
        {
            return Err(SpeechError::LimitExceeded);
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
        Ok(())
    }
    fn validate_word(&self, word: &LexicalWord) -> Result<()> {
        let mut cursor = word.phones.start;
        for syllable in &self.syllables[word.syllables.range()] {
            let nucleus = syllable.nucleus;
            if syllable.phones.start != cursor
                || syllable.phones.end > word.phones.end
                || nucleus < syllable.phones.start
                || nucleus >= syllable.phones.end
            {
                return Err(SpeechError::InvalidSpan);
            }
            let phone = self.phones[nucleus as usize];
            let onset = syllable.onset();
            let coda = syllable.coda();
            if !phone.phone.is_vowel()
                || phone.stress != Some(syllable.stress)
                || self.phones[onset.range()]
                    .iter()
                    .chain(&self.phones[coda.range()])
                    .any(|p| p.phone.is_vowel())
            {
                return Err(SpeechError::Pronunciation);
            }
            cursor = syllable.phones.end;
        }
        if cursor != word.phones.end {
            return Err(SpeechError::InvalidSpan);
        }
        Ok(())
    }
}
