//! Checked ARPAbet symbols and lexical stress.
use crate::{
    error::{Result, SpeechError},
    types::{ArpaPhone, EnglishPhone, Stress},
};

pub const SYMBOLS: [&str; 39] = [
    "AA", "AE", "AH", "AO", "AW", "AY", "B", "CH", "D", "DH", "EH", "ER", "EY", "F", "G", "HH",
    "IH", "IY", "JH", "K", "L", "M", "N", "NG", "OW", "OY", "P", "R", "S", "SH", "T", "TH", "UH",
    "UW", "V", "W", "Y", "Z", "ZH",
];
pub const PHONES: [ArpaPhone; 39] = [
    ArpaPhone::Aa,
    ArpaPhone::Ae,
    ArpaPhone::Ah,
    ArpaPhone::Ao,
    ArpaPhone::Aw,
    ArpaPhone::Ay,
    ArpaPhone::B,
    ArpaPhone::Ch,
    ArpaPhone::D,
    ArpaPhone::Dh,
    ArpaPhone::Eh,
    ArpaPhone::Er,
    ArpaPhone::Ey,
    ArpaPhone::F,
    ArpaPhone::G,
    ArpaPhone::Hh,
    ArpaPhone::Ih,
    ArpaPhone::Iy,
    ArpaPhone::Jh,
    ArpaPhone::K,
    ArpaPhone::L,
    ArpaPhone::M,
    ArpaPhone::N,
    ArpaPhone::Ng,
    ArpaPhone::Ow,
    ArpaPhone::Oy,
    ArpaPhone::P,
    ArpaPhone::R,
    ArpaPhone::S,
    ArpaPhone::Sh,
    ArpaPhone::T,
    ArpaPhone::Th,
    ArpaPhone::Uh,
    ArpaPhone::Uw,
    ArpaPhone::V,
    ArpaPhone::W,
    ArpaPhone::Y,
    ArpaPhone::Z,
    ArpaPhone::Zh,
];
impl ArpaPhone {
    pub fn is_vowel(self) -> bool {
        matches!(
            self,
            Self::Aa
                | Self::Ae
                | Self::Ah
                | Self::Ao
                | Self::Aw
                | Self::Ay
                | Self::Eh
                | Self::Er
                | Self::Ey
                | Self::Ih
                | Self::Iy
                | Self::Ow
                | Self::Oy
                | Self::Uh
                | Self::Uw
        )
    }
    pub fn symbol(self) -> &'static str {
        SYMBOLS[self as usize]
    }
}
impl EnglishPhone {
    pub fn new(phone: ArpaPhone, stress: Option<Stress>) -> Result<Self> {
        if phone.is_vowel() != stress.is_some() {
            return Err(SpeechError::Pronunciation);
        }
        Ok(Self { phone, stress })
    }
    pub fn phone(self) -> ArpaPhone {
        self.phone
    }
    pub fn stress(self) -> Option<Stress> {
        self.stress
    }
    pub fn parse(token: &str) -> Result<Self> {
        let (base, stress) = match token.as_bytes().last() {
            Some(b'0') => (&token[..token.len() - 1], Some(Stress::Unstressed)),
            Some(b'1') => (&token[..token.len() - 1], Some(Stress::Primary)),
            Some(b'2') => (&token[..token.len() - 1], Some(Stress::Secondary)),
            _ => (token, None),
        };
        let index = SYMBOLS
            .iter()
            .position(|s| *s == base)
            .ok_or(SpeechError::Pronunciation)?;
        let phone = PHONES[index];
        Self::new(
            phone,
            if phone.is_vowel() {
                Some(stress.unwrap_or(Stress::Unknown))
            } else {
                stress
            },
        )
    }
}
