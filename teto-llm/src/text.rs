//! Incremental channel filtering, bounded lexical normalization and segmentation.
use crate::*;

#[derive(Default, PartialEq)]
enum Channel {
    #[default]
    Answer,
    Header,
    Thought,
    Tool,
}
#[derive(Default)]
struct Filter {
    channel: Channel,
    marker: String,
    header: String,
}
impl Filter {
    fn character(&mut self, c: char) -> Result<Option<char>> {
        if c == '<' || !self.marker.is_empty() {
            if self.marker.len() >= 64 {
                return Err(failure(ErrorKind::Normalization));
            }
            self.marker.push(c);
            if c == '>' {
                self.control()?;
                self.marker.clear();
            }
            return Ok(None);
        }
        if self.channel == Channel::Header {
            if c == '\n' {
                if self.header != "thought" {
                    return Err(failure(ErrorKind::Normalization));
                }
                self.header.clear();
                self.channel = Channel::Thought;
            } else {
                if self.header.len() >= 16 {
                    return Err(failure(ErrorKind::Normalization));
                }
                self.header.push(c);
            }
            return Ok(None);
        }
        Ok((self.channel == Channel::Answer).then_some(c))
    }
    fn control(&mut self) -> Result<()> {
        self.channel = match (self.marker.as_str(), &self.channel) {
            ("<|channel>", Channel::Answer) => Channel::Header,
            ("<channel|>", Channel::Thought) => Channel::Answer,
            ("<|tool_call>", Channel::Answer) => Channel::Tool,
            ("<tool_call|>", Channel::Tool) => Channel::Answer,
            _ => return Err(failure(ErrorKind::Normalization)),
        };
        Ok(())
    }
    fn finish(&self) -> Result<()> {
        if self.channel != Channel::Answer || !self.marker.is_empty() {
            return Err(failure(ErrorKind::Normalization));
        }
        Ok(())
    }
}

#[derive(Default)]
struct Normalizer {
    word: String,
    ticks: usize,
    code: usize,
    line_start: bool,
    table: bool,
    link_end: bool,
    url: bool,
}
impl Normalizer {
    fn character(&mut self, c: char) -> Result<Option<String>> {
        if c == '`' {
            self.ticks = (self.ticks + 1).min(3);
            return if self.code == 0 {
                self.flush()
            } else {
                Ok(None)
            };
        }
        if self.ticks > 0 {
            if self.code == self.ticks {
                self.code = 0;
            } else if self.code == 0 {
                self.code = self.ticks;
            }
            self.ticks = 0;
        }
        if self.code > 0 {
            return Ok(None);
        }
        if self.link_end {
            self.link_end = false;
            if c == '(' {
                self.url = true;
            }
        }
        if self.url {
            if c == ')' {
                self.url = false;
            }
            return Ok(None);
        }
        if c == ']' {
            self.link_end = true;
            return Ok(None);
        }
        if self.line_start && !c.is_whitespace() {
            self.table = c == '|';
            self.line_start = false;
        }
        if c == '\n' {
            self.line_start = true;
        }
        if self.table {
            if c == '\n' {
                self.table = false;
            }
            return Ok(None);
        }
        let c = punctuation(c);
        if c.is_whitespace() {
            return self.flush();
        }
        if c.is_control() {
            return Err(failure(ErrorKind::Normalization));
        }
        if c == '*' || c == '_' || c == '#' || c == '[' {
            return Ok(None);
        }
        if is_emoji(c) {
            return self.flush();
        }
        if (!c.is_ascii() && !c.is_alphabetic()) || c == '|' {
            return Err(failure(ErrorKind::Normalization));
        }
        if self.word.len() >= policy::LATER_CHARS * 4 {
            return self.long_word();
        }
        self.word.push(c);
        Ok(None)
    }
    fn long_word(&self) -> Result<Option<String>> {
        if self.word.starts_with("http") || self.word.starts_with("www.") {
            Ok(None)
        } else {
            Err(failure(ErrorKind::Normalization))
        }
    }
    fn flush(&mut self) -> Result<Option<String>> {
        let raw = std::mem::take(&mut self.word);
        if raw.is_empty() {
            return Ok(None);
        }
        normalize_word(&raw)
    }
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1f000..=0x1faff | 0x2600..=0x27bf | 0xfe0f | 0x200d)
}

fn punctuation(c: char) -> char {
    match c {
        '’' | '‘' => '\'',
        '“' | '”' => '"',
        '—' | '–' => ',',
        '…' => '.',
        _ => c,
    }
}

fn normalize_word(raw: &str) -> Result<Option<String>> {
    if matches!(raw, "-" | "+") {
        return Ok(None);
    }
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("www.") {
        return Ok(None);
    }
    if let Some(expanded) = abbreviation(raw) {
        return Ok(Some(expanded));
    }
    let core = raw
        .trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '"' | '(' | ')'));
    if core.is_empty() {
        return Ok(None);
    }
    if core.chars().any(|c| c.is_ascii_digit()) {
        return normalize_number(raw, core).map(Some);
    }
    if [
        "kg", "mg", "km", "cm", "mm", "lbs", "lb", "oz", "mph", "kph", "kHz", "MHz", "GHz", "mL",
        "kB", "MB", "GB", "TB",
    ]
    .iter()
    .any(|unit| core.eq_ignore_ascii_case(unit))
    {
        return Err(failure(ErrorKind::Normalization));
    }
    if core.contains('.') {
        return Err(failure(ErrorKind::Normalization));
    }
    if core
        .chars()
        .any(|c| !(c.is_alphabetic() || matches!(c, '\'' | '-' | '.')))
    {
        return Err(failure(ErrorKind::Normalization));
    }
    Ok(Some(raw.to_owned()))
}

fn abbreviation(raw: &str) -> Option<String> {
    let core = raw.trim_matches([',', '!', '?', ';', ':', '"', '(', ')']);
    let expanded = match core {
        "Mr." => "Mister",
        "Mrs." => "Missus",
        "Ms." => "Miz",
        "Dr." => "Doctor",
        "e.g." => "for example",
        "i.e." => "that is",
        _ => return None,
    };
    let start = raw.find(core)?;
    Some(format!(
        "{}{}{}",
        &raw[..start],
        expanded,
        &raw[start + core.len()..]
    ))
}

fn normalize_number(raw: &str, core: &str) -> Result<String> {
    const SMALL: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    if !core.bytes().all(|b| b.is_ascii_digit()) {
        return Err(failure(ErrorKind::Normalization));
    }
    let value = core.parse::<usize>().ok().filter(|v| *v < SMALL.len());
    let expanded = if let Some(value) = value {
        SMALL[value].to_owned()
    } else {
        core.bytes()
            .map(|b| SMALL[(b - b'0') as usize])
            .collect::<Vec<_>>()
            .join(" ")
    };
    if expanded.len() > policy::LATER_CHARS * 4 {
        return Err(failure(ErrorKind::Normalization));
    }
    let start = raw
        .find(core)
        .ok_or_else(|| failure(ErrorKind::Normalization))?;
    Ok(format!(
        "{}{}{}",
        &raw[..start],
        expanded,
        &raw[start + core.len()..]
    ))
}

pub(crate) struct TextPipeline {
    filter: Filter,
    normalizer: Normalizer,
    utf8: Vec<u8>,
    clause: String,
    turn: TurnId,
    index: u32,
    total: usize,
}
impl TextPipeline {
    pub fn new(turn: TurnId) -> Self {
        Self {
            filter: Filter::default(),
            normalizer: Normalizer {
                line_start: true,
                ..Default::default()
            },
            utf8: Vec::with_capacity(4),
            clause: String::with_capacity(policy::LATER_CHARS * 4),
            turn,
            index: 0,
            total: 0,
        }
    }
    pub fn push(
        &mut self,
        bytes: &[u8],
        emit: &mut impl FnMut(Clause) -> Result<()>,
    ) -> Result<bool> {
        let mut answer = false;
        for byte in bytes {
            self.utf8.push(*byte);
            let c = match std::str::from_utf8(&self.utf8) {
                Ok(text) => text
                    .chars()
                    .next()
                    .ok_or_else(|| failure(ErrorKind::Normalization))?,
                Err(error) if error.error_len().is_none() && self.utf8.len() < 4 => continue,
                Err(_) => return Err(failure(ErrorKind::Normalization)),
            };
            self.utf8.clear();
            if let Some(c) = self.filter.character(c)? {
                answer |= !c.is_whitespace();
                if let Some(word) = self.normalizer.character(c)? {
                    self.words(&word, emit)?;
                }
            }
        }
        Ok(answer)
    }
    pub fn finish(&mut self, eos: bool, emit: &mut impl FnMut(Clause) -> Result<()>) -> Result<()> {
        if !eos {
            self.clause.clear();
            return Ok(());
        }
        self.filter.finish()?;
        if !self.utf8.is_empty() {
            return Err(failure(ErrorKind::Normalization));
        }
        if let Some(word) = self.normalizer.flush()? {
            self.words(&word, emit)?;
        }
        self.emit(emit)
    }
    fn words(&mut self, words: &str, emit: &mut impl FnMut(Clause) -> Result<()>) -> Result<()> {
        for word in words.split_whitespace() {
            let (max_words, max_chars) = if self.index == 0 {
                (policy::FIRST_WORDS, policy::FIRST_CHARS)
            } else {
                (policy::LATER_WORDS, policy::LATER_CHARS)
            };
            let count = self.clause.split_whitespace().count();
            let needed =
                self.clause.chars().count() + usize::from(count > 0) + word.chars().count();
            if count >= max_words || needed > max_chars {
                self.emit(emit)?;
            }
            let max_chars = if self.index == 0 {
                policy::FIRST_CHARS
            } else {
                policy::LATER_CHARS
            };
            if word.chars().count() > max_chars {
                return Err(failure(ErrorKind::Normalization));
            }
            if !self.clause.is_empty() {
                self.clause.push(' ');
            }
            self.clause.push_str(word);
            let end = word.trim_end_matches(['"', '\'', ')']);
            let sentence = end.ends_with(['.', '!', '?']);
            let words = self.clause.split_whitespace().count();
            let natural = end.ends_with([',', ';', ':'])
                && (words >= policy::MIN_WORDS || self.clause.chars().count() >= policy::MIN_CHARS);
            let first_target = self.index == 0 && words >= policy::FIRST_TARGET;
            if sentence || natural || first_target {
                self.emit(emit)?;
            }
        }
        Ok(())
    }
    fn emit(&mut self, emit: &mut impl FnMut(Clause) -> Result<()>) -> Result<()> {
        if self.clause.is_empty() {
            return Ok(());
        }
        self.total = self
            .total
            .checked_add(self.clause.len())
            .ok_or_else(|| failure(ErrorKind::Normalization))?;
        if self.total > policy::TURN_BYTES {
            return Err(failure(ErrorKind::Normalization));
        }
        let text = std::mem::replace(
            &mut self.clause,
            String::with_capacity(policy::LATER_CHARS * 4),
        );
        let sentence = text
            .trim_end_matches(['"', '\'', ')'])
            .ends_with(['.', '!', '?']);
        let clause = Clause::new(
            self.turn,
            ClauseIndex::new(self.index)?,
            text.try_into()?,
            if sentence {
                Boundary::Sentence
            } else {
                Boundary::Clause
            },
        )?;
        emit(clause)?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| failure(ErrorKind::Normalization))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
