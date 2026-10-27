//! Checked half-open spans.
use crate::error::{Result, SpeechError};

/// One half-open byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan {
    /// First byte.
    pub(crate) start: u32,
    /// One past the last byte.
    pub(crate) end: u32,
}

impl ByteSpan {
    /// Return the first byte.
    pub fn start(self) -> u32 {
        self.start
    }

    /// Return one past the last byte.
    pub fn end(self) -> u32 {
        self.end
    }

    /// Return the byte range.
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// Create a span.
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start > end {
            return Err(SpeechError::LimitExceeded);
        }
        Ok(Self { start, end })
    }
}

/// One half-open index span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexSpan {
    /// First index.
    pub(crate) start: u32,
    /// One past the last index.
    pub(crate) end: u32,
}

impl IndexSpan {
    /// Return the first index.
    pub fn start(self) -> u32 {
        self.start
    }

    /// Return one past the last index.
    pub fn end(self) -> u32 {
        self.end
    }

    /// Return the index range.
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// Create a span.
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start > end {
            return Err(SpeechError::LimitExceeded);
        }
        Ok(Self { start, end })
    }

    /// Return the span length.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Return true when the span is empty.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Check one span against a length.
pub(crate) fn check_span(span: IndexSpan, len: usize) -> Result<()> {
    if span.start > span.end || span.end as usize > len {
        return Err(SpeechError::InvalidSpan);
    }
    Ok(())
}
