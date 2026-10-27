//! Checked OTO decimal timing and optional local bank import.
use crate::error::{Result, SpeechError};
#[cfg(feature = "voicebank-wav")]
mod import;
#[cfg(feature = "voicebank-wav")]
pub use import::{BankLocation, ImportReport, ImportedBank, import_voicebank};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtoRow<'a> {
    pub source: &'a str,
    pub alias: &'a str,
    pub offset_us: u64,
    pub consonant_us: u64,
    pub cutoff_us: i64,
    pub preutterance_us: u64,
    pub overlap_us: i64,
}
/// Parse decimal milliseconds with at most three fractional digits.
pub fn decimal_us(value: &str) -> Result<i64> {
    let value = value.trim();
    let (negative, value) = match value.strip_prefix('-') {
        Some(v) => (true, v),
        None => (false, value),
    };
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    let whole: i64 = whole.parse().map_err(|_| SpeechError::LimitExceeded)?;
    let mut fractional = 0i64;
    for (i, b) in fraction.bytes().enumerate() {
        fractional += i64::from(b - b'0') * [100, 10, 1][i];
    }
    let result = whole
        .checked_mul(1000)
        .and_then(|v| v.checked_add(fractional))
        .ok_or(SpeechError::LimitExceeded)?;
    Ok(if negative { -result } else { result })
}
pub fn parse_row(line: &str) -> Result<OtoRow<'_>> {
    if line.len() > crate::voicebank::limits::MAX_OTO_LINE_BYTES {
        return Err(SpeechError::LimitExceeded);
    }
    let (source, rest) = line.split_once('=').ok_or(SpeechError::VoicebankInvalid)?;
    let mut fields = rest.split(',');
    let alias = fields.next().ok_or(SpeechError::VoicebankInvalid)?;
    if source.starts_with(['/', '\\'])
        || source.contains([':', '\0'])
        || source.split(['/', '\\']).any(|p| p == "..")
        || source.len() > crate::voicebank::limits::MAX_PATH_BYTES
        || source.is_empty()
        || alias.len() > crate::voicebank::limits::MAX_ALIAS_BYTES
        || alias.chars().any(char::is_control)
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    let mut values = [0i64; 5];
    for value in &mut values {
        *value = decimal_us(fields.next().ok_or(SpeechError::VoicebankInvalid)?)?;
    }
    if fields.next().is_some() || values[0] < 0 || values[1] < 0 || values[3] < 0 {
        return Err(SpeechError::VoicebankInvalid);
    }
    let alias = if alias.is_empty() {
        source.strip_suffix(".wav").unwrap_or(source)
    } else {
        alias
    };
    if alias.len() > crate::voicebank::limits::MAX_ALIAS_BYTES {
        return Err(SpeechError::LimitExceeded);
    }
    Ok(OtoRow {
        source,
        alias,
        offset_us: values[0] as u64,
        consonant_us: values[1] as u64,
        cutoff_us: values[2],
        preutterance_us: values[3] as u64,
        overlap_us: values[4],
    })
}

#[cfg(feature = "voicebank-wav")]
pub fn visit_rows(
    bytes: &[u8],
    mut consume: impl FnMut(u32, OtoRow<'_>) -> Result<()>,
) -> Result<()> {
    if bytes.len() > crate::voicebank::limits::MAX_OTO_BYTES {
        return Err(SpeechError::LimitExceeded);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => std::borrow::Cow::Borrowed(text.trim_start_matches('\u{feff}')),
        Err(_) => {
            let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
            if bad {
                return Err(SpeechError::VoicebankInvalid);
            }
            text
        }
    };
    let mut rows = 0;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        rows += 1;
        if rows > crate::voicebank::limits::MAX_OTO_ROWS {
            return Err(SpeechError::LimitExceeded);
        }
        consume(index as u32 + 1, parse_row(line)?)?;
    }
    Ok(())
}
