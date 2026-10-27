//! Voicebank and archive bounds.

/// Maximum compressed archive bytes.
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum archive or directory entries.
pub const MAX_ENTRIES: usize = 8192;
/// Maximum expanded archive bytes.
pub const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum OTO rows.
pub const MAX_OTO_ROWS: usize = 16384;
/// Maximum source recordings.
pub const MAX_SOURCES: usize = 4096;
/// Maximum pitch groups.
pub const MAX_PITCHES: usize = 128;
/// Maximum OTO file bytes.
pub const MAX_OTO_BYTES: usize = 8 * 1024 * 1024;
/// Maximum OTO line bytes.
pub const MAX_OTO_LINE_BYTES: usize = 4096;
/// Maximum alias bytes.
pub const MAX_ALIAS_BYTES: usize = 256;
/// Maximum path bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// Maximum WAV header bytes.
pub const MAX_WAV_HEADER_BYTES: u64 = 64 * 1024;
