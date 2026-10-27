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
    output: String,
    ticks: usize,
    code: usize,
    line_start: bool,
    table: bool,
    link_end: bool,
    url: bool,
}
impl Normalizer {
    /// Process one character. Return true when `output` holds a word.
    fn character(&mut self, c: char) -> Result<bool> {
        if c == '`' {
            self.ticks = (self.ticks + 1).min(3);
            return if self.code == 0 {
                self.flush()
            } else {
                Ok(false)
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
            return Ok(false);
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
            return Ok(false);
        }
        if c == ']' {
            self.link_end = true;
            return Ok(false);
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
            return Ok(false);
        }
        let c = punctuation(c);
        if c.is_whitespace() {
            return self.flush();
        }
        if c.is_control() {
            return Err(failure(ErrorKind::Normalization));
        }
        if c == '*' || c == '_' || c == '#' || c == '[' {
            return Ok(false);
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
        Ok(false)
    }

    fn long_word(&mut self) -> Result<bool> {
        if self.word.starts_with("http") || self.word.starts_with("www.") {
            self.word.clear();
            Ok(false)
        } else {
            Err(failure(ErrorKind::Normalization))
        }
    }

    /// Normalize the current word into `output`.
    fn flush(&mut self) -> Result<bool> {
        if self.word.is_empty() {
            return Ok(false);
        }
        let mut raw = std::mem::take(&mut self.word);
        self.output.clear();
        let result = normalize_word(&raw, &mut self.output);
        raw.clear();
        self.word = raw;
        result?;
        Ok(!self.output.is_empty())
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

fn normalize_word(raw: &str, out: &mut String) -> Result<()> {
    if matches!(raw, "-" | "+") {
        return Ok(());
    }
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("www.") {
        return Ok(());
    }
    if abbreviation(raw, out) {
        return Ok(());
    }
    let core = raw
        .trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '"' | '(' | ')'));
    if core.is_empty() {
        return Ok(());
    }
    if core.chars().any(|c| c.is_ascii_digit()) {
        return normalize_number(raw, core, out);
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
    out.push_str(raw);
    Ok(())
}

fn abbreviation(raw: &str, out: &mut String) -> bool {
    let core = raw.trim_matches([',', '!', '?', ';', ':', '"', '(', ')']);
    let expanded = match core {
        "Mr." => "Mister",
        "Mrs." => "Missus",
        "Ms." => "Miz",
        "Dr." => "Doctor",
        "e.g." => "for example",
        "i.e." => "that is",
        _ => return false,
    };
    let Some(start) = raw.find(core) else {
        return false;
    };
    out.push_str(&raw[..start]);
    out.push_str(expanded);
    out.push_str(&raw[start + core.len()..]);
    true
}

fn normalize_number(raw: &str, core: &str, out: &mut String) -> Result<()> {
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
    if !core.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(failure(ErrorKind::Normalization));
    }
    let start = raw
        .find(core)
        .ok_or_else(|| failure(ErrorKind::Normalization))?;
    out.push_str(&raw[..start]);
    let value = core
        .parse::<usize>()
        .ok()
        .filter(|value| *value < SMALL.len());
    if let Some(value) = value {
        out.push_str(SMALL[value]);
    } else {
        for (index, byte) in core.bytes().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push_str(SMALL[(byte - b'0') as usize]);
        }
    }
    out.push_str(&raw[start + core.len()..]);
    if out.len() > policy::LATER_CHARS * 4 {
        return Err(failure(ErrorKind::Normalization));
    }
    Ok(())
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
                if self.normalizer.character(c)? {
                    self.process_output(emit)?;
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
        if self.normalizer.flush()? {
            self.process_output(emit)?;
        }
        self.emit(emit)
    }
    fn process_output(&mut self, emit: &mut impl FnMut(Clause) -> Result<()>) -> Result<()> {
        if self.normalizer.output.is_empty() {
            return Ok(());
        }
        let output = std::mem::take(&mut self.normalizer.output);
        let result = self.words(&output, emit);
        self.normalizer.output = output;
        self.normalizer.output.clear();
        result
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
        let clause = Clause::new(self.turn, ClauseIndex::new(self.index)?, text.try_into()?)?;
        emit(clause)?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| failure(ErrorKind::Normalization))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn render(chunks: &[&[u8]], eos: bool) -> Result<Vec<Clause>> {
        let mut text = TextPipeline::new(TurnId::new(1).unwrap());
        let mut clauses = Vec::new();
        let mut emit = |clause| {
            clauses.push(clause);
            Ok(())
        };
        for bytes in chunks {
            text.push(bytes, &mut emit)?;
        }
        text.finish(eos, &mut emit)?;
        Ok(clauses)
    }
    fn strings(clauses: &[Clause]) -> Vec<&str> {
        clauses.iter().map(Clause::text).collect()
    }
    #[test]
    fn normalization_v1() {
        let clauses = render(
            &[b"# **Hello**, Dr. Smith. I have 12 apples and 42 pears."],
            true,
        )
        .unwrap();
        assert_eq!(
            strings(&clauses).join(" "),
            "Hello, Doctor Smith. I have twelve apples and four two pears."
        );
        assert!(render(&[b"It costs $20."], true).is_err());
        assert!(render(&[b"It is 3.14."], true).is_err());
        assert!(render(&[b"The 21st."], true).is_err());
        assert!(render(&[b"On 2026-10-27."], true).is_err());
        assert!(render(&[b"It weighs 10kg."], true).is_err());
        assert!(render(&[b"It weighs 10 kg."], true).is_err());
    }
    #[test]
    fn reasoning_and_control_suppressed() {
        let input = b"<|channel>thought\nSECRET hidden analysis.<channel|>Hello there. <|tool_call>SECRET tool<tool_call|>Goodbye.";
        for split in 0..=input.len() {
            let clauses = render(&[&input[..split], &input[split..]], true).unwrap();
            assert_eq!(strings(&clauses), ["Hello there.", "Goodbye."]);
        }
        assert!(render(&[b"<|channel>unknown\nsecret"], true).is_err());
        assert!(render(&[b"<|channel>thought\nnever closed"], true).is_err());
        assert!(render(&[b"<unknown>secret"], true).is_err());
    }
    #[test]
    fn clause_order() {
        let result = render(&[b"Hello. One, two and three. Goodbye."], true).unwrap();
        for (index, clause) in result.iter().enumerate() {
            assert_eq!(clause.index().get() as usize, index);
        }
    }
    #[test]
    fn every_clause_has_turn() {
        for clause in render(&[b"Hello. Goodbye."], true).unwrap() {
            assert_eq!(clause.turn().get(), 1);
        }
    }
    #[test]
    fn output_token_bound() {
        let clauses = render(&[b"A complete sentence. An unfinished tail"], false).unwrap();
        assert_eq!(strings(&clauses), ["A complete sentence."]);
    }
    #[test]
    fn normalization_omits_code_tables_urls_emoji() {
        let input = "Hello.\n```rust\nsecret code\n```\n| secret | table |\nVisit https://example.com now. 🙂 ❤️ [Read this](https://example.com).";
        let clauses = render(&[input.as_bytes()], true).unwrap();
        let output = strings(&clauses).join(" ");
        assert!(!output.contains("secret"));
        assert!(!output.contains("http"));
        assert!(output.contains("Read this"));
        for split in 0..=input.len() {
            let other = render(
                &[&input.as_bytes()[..split], &input.as_bytes()[split..]],
                true,
            )
            .unwrap();
            assert_eq!(strings(&other).join(" "), output);
        }
    }
    proptest! {
        #[test]
        fn chunking_is_invariant(words in prop::collection::vec("[a-z]{1,10}", 1..100), chunk in 1usize..40) {
            let input = format!("{}.", words.join(" "));
            let chunks: Vec<_> = input.as_bytes().chunks(chunk).collect();
            match (render(&chunks, true), render(&[input.as_bytes()], true)) {
                (Ok(a), Ok(b)) => {
                    prop_assert_eq!(strings(&a), strings(&b));
                    for clause in &a {
                        let limit = if clause.index().get() == 0 { policy::FIRST_CHARS } else { policy::LATER_CHARS };
                        prop_assert!(clause.text().chars().count() <= limit);
                    }
                }
                (Err(a), Err(b)) => prop_assert_eq!(a.kind(), b.kind()),
                _ => prop_assert!(false, "chunking changed acceptance"),
            }
        }
    }

    #[test]
    fn unicode_prose_keeps_scalar_boundaries() {
        let input = "Renée visits a café. She says ‘Hello’…";
        let expected = "Renée visits a café. She says 'Hello'.";
        for split in 0..=input.len() {
            let clauses = render(
                &[&input.as_bytes()[..split], &input.as_bytes()[split..]],
                true,
            )
            .unwrap();
            assert_eq!(strings(&clauses).join(" "), expected);
        }
    }

    #[test]
    fn fresh_clause_rechecks_minimum_preference() {
        let input = format!("Hello. {} a b However, this continues.", "a".repeat(230));
        let clauses = render(&[input.as_bytes()], true).unwrap();
        assert_eq!(clauses.last().unwrap().text(), "However, this continues.");
    }

    #[test]
    fn abbreviation_wrappers_preserve_punctuation() {
        let result = render(&[b"Ask (Dr.) Smith; e.g., say hello."], true).unwrap();
        assert_eq!(
            strings(&result).join(" "),
            "Ask (Doctor) Smith; for example, say hello."
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_are_bounded(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
            if let Ok(clauses) = render(&[&bytes], true) {
                for clause in clauses {
                    prop_assert!(clause.text().chars().count() <= policy::LATER_CHARS);
                    prop_assert!(!clause.text().contains('<'));
                }
            }
        }
    }
}
