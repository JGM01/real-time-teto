use crate::{ErrorKind, Result, failure, policy};
use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU64, sync::Arc};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A profile in the built-in catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub enum ProfileId {
    #[serde(rename = "gemma4-26b-qat-q4_0-macos-v1")]
    Desktop,
    #[serde(rename = "gemma4-e2b-qat-q4_0-ios-spike-v1")]
    PhoneSpike,
}
impl ProfileId {
    pub fn desktop() -> Self {
        Self::Desktop
    }
    pub fn phone_spike() -> Self {
        Self::PhoneSpike
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => policy::PROFILE_ID,
            Self::PhoneSpike => policy::PHONE_PROFILE,
        }
    }
    pub fn parse(id: &str) -> Result<Self> {
        match id {
            policy::PROFILE_ID => Ok(Self::Desktop),
            policy::PHONE_PROFILE => Ok(Self::PhoneSpike),
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
    fn try_from(value: u64) -> Result<Self> {
        Self::new(value)
    }
}
impl From<TurnId> for u64 {
    fn from(value: TurnId) -> Self {
        value.get()
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
    fn try_from(value: u32) -> Result<Self> {
        Self::new(value)
    }
}
impl From<ClauseIndex> for u32 {
    fn from(value: ClauseIndex) -> Self {
        value.get()
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
    fn try_from(value: InputData) -> Result<Self> {
        Self::new(value.text, value.truncated)
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UserInput([redacted])")
    }
}

#[derive(Clone, Deserialize)]
#[serde(try_from = "String")]
pub struct SpeakableText(Arc<str>);
impl Serialize for SpeakableText {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
impl SpeakableText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0.to_string()
    }
}
impl TryFrom<String> for SpeakableText {
    type Error = crate::Error;
    fn try_from(text: String) -> Result<Self> {
        if text.is_empty()
            || text.len() > policy::LATER_CHARS * 4
            || text.trim() != text
            || text.contains("  ")
            || !text.chars().any(char::is_alphabetic)
            || text.chars().any(|character| {
                !(character.is_alphabetic()
                    || matches!(
                        character,
                        ' ' | '.' | ',' | '!' | '?' | ';' | ':' | '\'' | '"' | '(' | ')' | '-'
                    ))
            })
        {
            return Err(failure(ErrorKind::Normalization));
        }
        Ok(Self(Arc::from(text)))
    }
}
impl fmt::Debug for SpeakableText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SpeakableText([redacted])")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "ClauseData")]
pub struct Clause {
    turn: TurnId,
    index: ClauseIndex,
    text: SpeakableText,
}
#[derive(Deserialize)]
struct ClauseData {
    turn: TurnId,
    index: ClauseIndex,
    text: SpeakableText,
}
impl TryFrom<ClauseData> for Clause {
    type Error = crate::Error;
    fn try_from(value: ClauseData) -> Result<Self> {
        Self::new(value.turn, value.index, value.text)
    }
}
impl Clause {
    pub fn new(turn: TurnId, index: ClauseIndex, text: SpeakableText) -> Result<Self> {
        let (words, characters) = if index.get() == 0 {
            (policy::FIRST_WORDS, policy::FIRST_CHARS)
        } else {
            (policy::LATER_WORDS, policy::LATER_CHARS)
        };
        if text.as_str().chars().count() > characters
            || text.as_str().split_whitespace().count() > words
        {
            return Err(failure(ErrorKind::Normalization));
        }
        Ok(Self { turn, index, text })
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
    pub fn is_sentence(&self) -> bool {
        self.text
            .as_str()
            .trim_end_matches(['"', '\'', ')'])
            .ends_with(['.', '!', '?'])
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
    /// Explicit orchestration-only measurement. Never valid for persistent production history.
    TestSink,
}

/// How many delivered clauses may wait for acknowledgment.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryWindow {
    /// One delivered clause.
    #[default]
    One,
    /// Two delivered clauses for lookahead.
    Two,
}

impl DeliveryWindow {
    /// Return the window size.
    pub fn get(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }
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

/// The caller selects no-journal mode only when it is explicit.
#[derive(Clone, Serialize, Deserialize)]
pub enum JournalAccess {
    Disabled,
    Encrypted {
        directory: std::path::PathBuf,
        key: SecretKey,
    },
}
impl fmt::Debug for JournalAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JournalAccess([redacted])")
    }
}
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretKey([redacted])")
    }
}

/// The device and OS recorded for cache qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    pub device: String,
    pub os: String,
}
impl HostIdentity {
    pub fn new(device: impl Into<String>, os: impl Into<String>) -> Self {
        Self {
            device: device.into(),
            os: os.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalIdentity {
    pub journal: [u8; 16],
    pub key: [u8; 16],
}

/// Measurements for one turn. Time values use microseconds.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub cache_mode: CacheMode,
    pub cache_reason: CacheReason,
    pub context_tokens: u32,
    pub headroom_tokens: u32,
    pub prompt_tokens: u32,
    pub cached_prefix_tokens: u32,
    pub evaluated_prompt_tokens: u32,
    pub generated_tokens: u32,
    pub evicted_exchanges: u32,
    pub evicted_tokens: u32,
    pub context_rebuilds: u32,
    pub prompt_render_us: u64,
    pub prefill_us: u64,
    pub decode_us: u64,
    pub first_answer_us: Option<u64>,
    pub first_clause_us: Option<u64>,
    pub last_clause_us: Option<u64>,
    pub queue_wait_us: u64,
    pub journal_us: u64,
    pub cancel_us: Option<u64>,
    pub clauses: u32,
    pub characters: u32,
    pub queue_high_water: u32,
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FinalizedTurn([redacted])")
    }
}
