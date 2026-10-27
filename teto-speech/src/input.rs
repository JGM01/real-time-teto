//! Speech identity and bounded clause text.
use std::{fmt, num::NonZeroU64, sync::Arc};

use crate::error::{Result, SpeechError};

/// Maximum clauses in one turn.
pub const MAX_CLAUSES: u32 = 512;
/// Maximum input scalars per clause.
pub const MAX_INPUT_SCALARS: usize = 240;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Identity of one speech item across every stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpeechKey {
    epoch: u64,
    turn: NonZeroU64,
    clause: u32,
}

impl SpeechKey {
    /// Create a key.
    pub fn new(epoch: u64, turn: u64, clause: u32) -> Result<Self> {
        let turn = NonZeroU64::new(turn).ok_or(SpeechError::InvalidInput)?;
        if clause >= MAX_CLAUSES {
            return Err(SpeechError::LimitExceeded);
        }
        Ok(Self {
            epoch,
            turn,
            clause,
        })
    }

    /// Return the generation.
    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    /// Return the turn.
    pub const fn turn(self) -> NonZeroU64 {
        self.turn
    }

    /// Return the clause index.
    pub const fn clause(self) -> u32 {
        self.clause
    }
}

/// One bounded English clause.
#[derive(Clone)]
pub struct SpeechInput {
    key: SpeechKey,
    text: Arc<str>,
    revision: u64,
}

impl SpeechInput {
    /// Create an input from normalized clause text.
    pub fn new(key: SpeechKey, text: impl AsRef<str>) -> Result<Self> {
        let text = text.as_ref();
        if text.trim().is_empty() {
            return Err(SpeechError::InvalidInput);
        }
        let mut revision = FNV_OFFSET;
        let mut scalars = 0usize;
        for character in text.chars() {
            if character.is_control() {
                return Err(SpeechError::InvalidInput);
            }
            scalars += 1;
            if scalars > MAX_INPUT_SCALARS {
                return Err(SpeechError::LimitExceeded);
            }
            revision = hash_byte(revision, character as u32);
        }
        Ok(Self {
            key,
            text: Arc::from(text),
            revision,
        })
    }

    /// Return the key.
    pub const fn key(&self) -> SpeechKey {
        self.key
    }

    /// Return the text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return the text revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

pub(crate) fn hash_byte(hash: u64, value: u32) -> u64 {
    let mut hash = hash;
    for byte in value.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

impl fmt::Debug for SpeechInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpeechInput")
            .field("key", &self.key)
            .field("text", &"[redacted]")
            .finish()
    }
}
