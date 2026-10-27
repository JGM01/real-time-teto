//! Voicebank identifiers.

/// Voicebank alias inventory identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AliasInventoryId(pub [u8; 32]);

/// One alias identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AliasIndex(pub u32);

/// One recorded pitch group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PitchId(pub u32);

/// One OTO row identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OtoRowId(pub u32);

/// One source recording identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u32);
