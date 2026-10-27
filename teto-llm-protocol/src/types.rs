use crate::{ErrorKind, Result, failure, policy};
use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU64};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A checked catalog identity. A bare path cannot name a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProfileId(&'static str);
impl ProfileId {
    pub fn desktop() -> Self {
        Self(policy::PROFILE_ID)
    }
    pub fn phone_spike() -> Self {
        Self(policy::PHONE_PROFILE)
    }
    pub fn as_str(self) -> &'static str {
        self.0
    }
    pub fn parse(id: &str) -> Result<Self> {
        match id {
            policy::PROFILE_ID => Ok(Self::desktop()),
            policy::PHONE_PROFILE => Ok(Self::phone_spike()),
            _ => Err(failure(ErrorKind::Profile)),
        }
    }
}
impl TryFrom<String> for ProfileId {
    type Error = crate::Error;
    fn try_from(id: String) -> Result<Self> {
        Self::parse(&id)
    }
}
impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::parse(&id).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputLanguage {
    #[serde(rename = "en")]
    English,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct TurnId(NonZeroU64);
impl TurnId {
    pub fn new(value: u64) -> Result<Self> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| failure(ErrorKind::Protocol))
    }
    pub fn get(self) -> u64 {
        self.0.get()
    }
    pub fn next(self) -> Result<Self> {
        Self::new(
            self.get()
                .checked_add(1)
                .ok_or_else(|| failure(ErrorKind::Worker))?,
        )
    }
}
impl TryFrom<u64> for TurnId {
    type Error = crate::Error;
    fn try_from(v: u64) -> Result<Self> {
        Self::new(v)
    }
}
impl From<TurnId> for u64 {
    fn from(v: TurnId) -> Self {
        v.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ClauseIndex(u32);
impl ClauseIndex {
    pub fn new(value: u32) -> Result<Self> {
        if value as usize >= policy::TURN_CLAUSES {
            return Err(failure(ErrorKind::Protocol));
        }
        Ok(Self(value))
    }
    pub fn get(self) -> u32 {
        self.0
    }
}
impl TryFrom<u32> for ClauseIndex {
    type Error = crate::Error;
    fn try_from(v: u32) -> Result<Self> {
        Self::new(v)
    }
}
impl From<ClauseIndex> for u32 {
    fn from(v: ClauseIndex) -> Self {
        v.get()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(try_from = "InputData")]
pub struct UserInput {
    text: String,
    truncated: bool,
}
#[derive(Deserialize)]
struct InputData {
    text: String,
    truncated: bool,
}
impl TryFrom<InputData> for UserInput {
    type Error = crate::Error;
    fn try_from(v: InputData) -> Result<Self> {
        Self::new(v.text, v.truncated)
    }
}
impl UserInput {
    pub fn new(text: String, truncated: bool) -> Result<Self> {
        if text.trim().is_empty()
            || text.len() > policy::INPUT_BYTES
            || text.chars().any(char::is_control)
        {
            return Err(failure(ErrorKind::Config));
        }
        Ok(Self { text, truncated })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}
impl fmt::Debug for UserInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UserInput([redacted])")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SpeakableText(String);
impl SpeakableText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
}
impl TryFrom<String> for SpeakableText {
    type Error = crate::Error;
    fn try_from(text: String) -> Result<Self> {
        if text.is_empty()
            || text.len() > policy::LATER_CHARS * 4
            || text.trim() != text
            || text.contains("  ")
            || !text.chars().any(|c| c.is_alphabetic())
            || text.chars().any(|c| {
                !(c.is_alphabetic()
                    || matches!(
                        c,
                        ' ' | '.' | ',' | '!' | '?' | ';' | ':' | '\'' | '"' | '(' | ')' | '-'
                    ))
            })
        {
            return Err(failure(ErrorKind::Normalization));
        }
        Ok(Self(text))
    }
}
impl fmt::Debug for SpeakableText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SpeakableText([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Boundary {
    Clause,
    Sentence,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(try_from = "ClauseData")]
pub struct Clause {
    turn: TurnId,
    index: ClauseIndex,
    text: SpeakableText,
    boundary: Boundary,
}
#[derive(Deserialize)]
struct ClauseData {
    turn: TurnId,
    index: ClauseIndex,
    text: SpeakableText,
    boundary: Boundary,
}
impl TryFrom<ClauseData> for Clause {
    type Error = crate::Error;
    fn try_from(v: ClauseData) -> Result<Self> {
        Self::new(v.turn, v.index, v.text, v.boundary)
    }
}
impl Clause {
    pub fn new(
        turn: TurnId,
        index: ClauseIndex,
        text: SpeakableText,
        boundary: Boundary,
    ) -> Result<Self> {
        let sentence = text
            .as_str()
            .trim_end_matches(['"', '\'', ')'])
            .ends_with(['.', '!', '?']);
        if sentence != (boundary == Boundary::Sentence) {
            return Err(failure(ErrorKind::Normalization));
        }
        let (words, chars) = if index.get() == 0 {
            (policy::FIRST_WORDS, policy::FIRST_CHARS)
        } else {
            (policy::LATER_WORDS, policy::LATER_CHARS)
        };
        if text.as_str().chars().count() > chars || text.as_str().split_whitespace().count() > words
        {
            return Err(failure(ErrorKind::Normalization));
        }
        Ok(Self {
            turn,
            index,
            text,
            boundary,
        })
    }
    pub fn turn(&self) -> TurnId {
        self.turn
    }
    pub fn index(&self) -> ClauseIndex {
        self.index
    }
    pub fn text(&self) -> &str {
        self.text.as_str()
    }
    pub fn boundary(&self) -> Boundary {
        self.boundary
    }
    pub fn into_text(self) -> String {
        self.text.into_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationStop {
    Eos,
    TokenLimit,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStop {
    Complete,
    Interrupted,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptSource {
    Playback,
    Display,
    /// Explicit orchestration-only measurement; never valid for persistent production history.
    TestSink,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheMode {
    #[default]
    FullRebuild,
    VerifiedPrefixReuse,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheReason {
    #[default]
    Cold,
    Disabled,
    Reused,
    CanonicalSuffix,
    Eviction,
    Reset,
    RemovalFailed,
    GenerationFailed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ClausePlayed {
    pub turn: TurnId,
    pub index: ClauseIndex,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClauseEvent {
    Clause(Clause),
    End {
        turn: TurnId,
        reason: GenerationStop,
    },
    Cancelled {
        turn: TurnId,
    },
}

/// Only explicitly selected no-journal mode avoids key access.
#[derive(Serialize, Deserialize)]
pub enum JournalAccess {
    Disabled,
    Encrypted {
        directory: std::path::PathBuf,
        key: SecretKey,
    },
}
impl fmt::Debug for JournalAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JournalAccess([redacted])")
    }
}
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; 32]);
impl SecretKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalIdentity {
    pub journal: [u8; 16],
    pub key: [u8; 16],
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub cache_mode: CacheMode,
    pub cache_reason: CacheReason,
    pub worker_name: String,
    pub prompt_tokens: u32,
    pub generated_tokens: u32,
    pub cached_prefix_tokens: u32,
    pub uncached_prompt_tokens: u32,
    pub evaluated_prompt_tokens: u32,
    pub evicted_turns: u32,
    pub evicted_tokens: u32,
    pub context_tokens: u32,
    pub headroom_tokens: u32,
    pub prompt_ms: f64,
    pub prompt_render_ms: f64,
    pub first_answer_ms: Option<f64>,
    pub first_clause_ms: Option<f64>,
    pub last_clause_ms: Option<f64>,
    pub generation_ms: f64,
    pub queue_wait_ms: f64,
    pub queue_high_water: usize,
    pub cancel_ms: Option<f64>,
    pub journal_ms: f64,
    pub clauses: u32,
    pub characters: u32,
}

/// A finalized record contains no unacknowledged assistant text.
#[derive(Serialize, Deserialize)]
pub struct FinalizedTurn {
    pub schema_version: u32,
    pub policy_version: u32,
    pub session: [u8; 16],
    pub turn: TurnId,
    pub profile: String,
    pub user: UserInput,
    pub assistant: Vec<String>,
    pub generation_stop: GenerationStop,
    pub playback_stop: PlaybackStop,
    pub partial: bool,
    pub receipt_source: ReceiptSource,
    pub metrics: TurnMetrics,
}
impl fmt::Debug for FinalizedTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FinalizedTurn([redacted])")
    }
}
