//! Text, lexicon and model bounds.

/// Maximum phones per clause.
pub const MAX_PHONES: usize = 512;
/// Maximum words per clause.
pub const MAX_WORDS: usize = 64;
/// Maximum syllables per clause.
pub const MAX_SYLLABLES: usize = 256;
/// Maximum dictionary bytes.
pub const MAX_DICTIONARY_BYTES: usize = 8 * 1024 * 1024;
/// Maximum dictionary line bytes.
pub const MAX_DICTIONARY_LINE_BYTES: usize = 4096;
/// Maximum lexicon rows.
pub const MAX_LEXICON_ROWS: usize = 200_000;
/// Maximum lexicon phones.
pub const MAX_LEXICON_PHONES: usize = 2_000_000;
/// Maximum pronunciation variants per word.
pub const MAX_VARIANTS: usize = 16;
/// Maximum phones per word.
pub const MAX_WORD_PHONES: usize = 64;
/// Maximum model bytes.
pub const MAX_MODEL_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum word ASCII bytes.
pub const MAX_WORD_ASCII: usize = 64;
/// Maximum decoder steps.
pub const MAX_G2P_STEPS: usize = 20;
