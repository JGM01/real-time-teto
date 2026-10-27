//! Pinned CMU pronunciations in contiguous spelling and phone pools.
use crate::{
    error::{Result, SpeechError},
    phone::EnglishPhone,
    span::IndexSpan,
    text::limits::*,
};
use sha2::{Digest, Sha256};
const DICTIONARY: &[u8] = include_bytes!("../../assets/cmudict/cmudict.dict");
const SHA256: [u8; 32] = [
    0x81, 0x91, 0x78, 0x43, 0xc7, 0xf4, 0x4c, 0xe2, 0xb0, 0x94, 0xac, 0x63, 0x87, 0x3c, 0x2c, 0x7a,
    0x4c, 0xf8, 0x02, 0x04, 0x07, 0x92, 0xc4, 0x55, 0xba, 0x3c, 0xa4, 0x06, 0x89, 0x1c, 0x3d, 0x22,
];
#[derive(Debug)]
struct Pronunciation {
    word: IndexSpan,
    ordinal: u32,
    phones: IndexSpan,
    line: u32,
}
pub struct Lexicon {
    spelling: String,
    rows: Vec<Pronunciation>,
    phones: Vec<EnglishPhone>,
    revision: [u8; 32],
}
impl Lexicon {
    pub fn embedded() -> Result<Self> {
        let revision: [u8; 32] = Sha256::digest(DICTIONARY).into();
        if revision != SHA256 {
            return Err(SpeechError::ModelMismatch);
        }
        Self::parse_with_revision(DICTIONARY, revision)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_revision(bytes, Sha256::digest(bytes).into())
    }
    fn parse_with_revision(bytes: &[u8], revision: [u8; 32]) -> Result<Self> {
        if bytes.len() > MAX_DICTIONARY_BYTES {
            return Err(SpeechError::LimitExceeded);
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| SpeechError::LexiconInvalid { line: 0 })?;
        let mut value = Self {
            spelling: String::new(),
            rows: Vec::new(),
            phones: Vec::new(),
            revision,
        };
        for (i, line) in text.lines().enumerate() {
            if line.len() > MAX_DICTIONARY_LINE_BYTES {
                return Err(SpeechError::LimitExceeded);
            }
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with(";;;") {
                continue;
            }
            value.add_row(line, i as u32 + 1)?;
        }
        value.rows.sort_by(|a, b| {
            value.spelling[a.word.range()]
                .cmp(&value.spelling[b.word.range()])
                .then(a.ordinal.cmp(&b.ordinal))
        });
        value.check_duplicates()?;
        Ok(value)
    }
    fn add_row(&mut self, line: &str, number: u32) -> Result<()> {
        if self.rows.len() >= MAX_LEXICON_ROWS {
            return Err(SpeechError::LimitExceeded);
        }
        let mut parts = line.split_whitespace();
        let raw = parts
            .next()
            .ok_or(SpeechError::LexiconInvalid { line: number })?;
        let (word, ordinal) =
            parse_word(raw).map_err(|_| SpeechError::LexiconInvalid { line: number })?;
        if word.len() > MAX_WORD_ASCII || !word.is_ascii() {
            return Err(SpeechError::LexiconInvalid { line: number });
        }
        let start = self.phones.len();
        for token in parts {
            if self.phones.len() - start >= MAX_WORD_PHONES
                || self.phones.len() >= MAX_LEXICON_PHONES
            {
                return Err(SpeechError::LimitExceeded);
            }
            self.phones.push(
                EnglishPhone::parse(token)
                    .map_err(|_| SpeechError::LexiconInvalid { line: number })?,
            );
        }
        if start == self.phones.len() {
            return Err(SpeechError::LexiconInvalid { line: number });
        }
        let word_start = self.spelling.len();
        self.spelling
            .extend(word.bytes().map(|b| b.to_ascii_lowercase() as char));
        self.rows.push(Pronunciation {
            word: IndexSpan::new(word_start as u32, self.spelling.len() as u32)?,
            ordinal,
            phones: IndexSpan::new(start as u32, self.phones.len() as u32)?,
            line: number,
        });
        Ok(())
    }
    fn check_duplicates(&mut self) -> Result<()> {
        let mut keep = Vec::<Pronunciation>::with_capacity(self.rows.len());
        let mut group = 0;
        for row in self.rows.drain(..) {
            if keep
                .last()
                .is_none_or(|p| self.spelling[p.word.range()] != self.spelling[row.word.range()])
            {
                group = keep.len();
            }
            let preceding = &keep[group..];
            if preceding.iter().any(|p| {
                p.ordinal == row.ordinal
                    && self.phones[p.phones.range()] != self.phones[row.phones.range()]
            }) {
                return Err(SpeechError::LexiconInvalid { line: row.line });
            }
            if preceding
                .iter()
                .any(|p| self.phones[p.phones.range()] == self.phones[row.phones.range()])
            {
                continue;
            }
            if preceding.len() >= MAX_VARIANTS {
                return Err(SpeechError::LimitExceeded);
            }
            keep.push(row);
        }
        self.rows = keep;
        Ok(())
    }
    pub fn lookup(&self, word: &str) -> Option<(&[EnglishPhone], u32)> {
        let start = self
            .rows
            .partition_point(|r| &self.spelling[r.word.range()] < word);
        let row = self.rows.get(start)?;
        if &self.spelling[row.word.range()] != word {
            return None;
        }
        let count = self.rows[start..].partition_point(|r| &self.spelling[r.word.range()] == word);
        Some((&self.phones[row.phones.range()], count as u32))
    }
    pub fn revision(&self) -> [u8; 32] {
        self.revision
    }
}
fn parse_word(raw: &str) -> Result<(&str, u32)> {
    if raw.ends_with(')') {
        let (word, n) = raw.rsplit_once('(').ok_or(SpeechError::Pronunciation)?;
        let ordinal = n[..n.len() - 1]
            .parse()
            .map_err(|_| SpeechError::Pronunciation)?;
        if ordinal < 2 {
            return Err(SpeechError::Pronunciation);
        }
        Ok((word, ordinal))
    } else {
        Ok((raw, 1))
    }
}
