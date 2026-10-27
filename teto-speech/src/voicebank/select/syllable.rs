//! OpenUtau English X-SAMPA syllable candidate order.
use super::*;
type ClusterMatch = (UnitId, OtoEntry, Substitutions, usize);
struct Context {
    start: usize,
    first: usize,
    nucleus: usize,
    previous: usize,
}
impl Context {
    fn count(&self) -> usize {
        self.nucleus - self.first
    }
    fn current(&self) -> usize {
        self.count() - self.previous
    }
}
impl Selection<'_> {
    pub(super) fn port_syllable(
        &mut self,
        start: usize,
        nucleus: usize,
        initial: bool,
        word_start: usize,
    ) -> Result<()> {
        if start == nucleus {
            return self.starting_vowel(nucleus);
        }
        let first = if initial { start } else { start + 1 };
        if first == nucleus {
            return self.vv(start, nucleus);
        }
        let context = Context {
            start,
            first,
            nucleus,
            previous: word_start.saturating_sub(first).min(nucleus - first),
        };
        if initial {
            self.starting(&context)
        } else {
            self.medial(&context)
        }
    }
    fn starting_vowel(&mut self, n: usize) -> Result<()> {
        let v = self.atoms[n].symbol.text();
        if self.choose(&[alias!("- {v}"), alias!("{v}")], n, n + 1)? {
            return Ok(());
        }
        self.missing(n, n + 1)
    }
    fn starting(&mut self, c: &Context) -> Result<()> {
        let v = self.atoms[c.nucleus].symbol.text();
        let last = self.atoms[c.nucleus - 1].symbol.text();
        let all = self.join(c.first, c.nucleus, "")?;
        if self.choose(&[alias!("- {all}{v}")], c.first, c.nucleus + 1)? {
            return Ok(());
        }
        let mut base = self.find_candidates(&[
            alias!("_{last}{v}"),
            alias!("{last} {v}"),
            alias!("{last}{v}"),
        ])?;
        if c.count() == 1 {
            base = self.find_candidates(&[alias!("{last} {v}"), alias!("{last}{v}")])?;
        }
        let Some(mut base) = base else {
            return self.missing(c.first, c.nucleus + 1);
        };
        let mut first = 0;
        let mut base_start = c.nucleus - 1;
        for length in (2..=c.count()).rev() {
            let joined = self.join(c.first, c.first + length, "")?;
            if self.choose(&[alias!("- {joined}")], c.first, c.first + length)? {
                first = length - 1;
                break;
            }
        }
        if first == 0 {
            let a = self.atoms[c.first].symbol.text();
            let _ = self.choose(&[alias!("- {a}")], c.first, c.first + 1)?;
        }
        if first + 1 < c.count() {
            let joined = self.join(c.first + first, c.nucleus, "")?;
            if let Some(found) = self.find_candidates(&[alias!("{joined}{v}")])? {
                base = found;
                base_start = c.first + first;
            }
        }
        self.consonants(c, first, base_start, &mut base)?;
        self.add(base.0, base.1, base_start, c.nucleus + 1, base.2)
    }
    fn medial(&mut self, c: &Context) -> Result<()> {
        let p = self.atoms[c.start].symbol.text();
        let v = self.atoms[c.nucleus].symbol.text();
        let first = self.atoms[c.first].symbol.text();
        let last = self.atoms[c.nucleus - 1].symbol.text();
        if c.count() == 1 {
            if c.previous == 1
                && self.choose(&[alias!("{p}{first} {v}")], c.start, c.nucleus + 1)?
            {
                return Ok(());
            }
            if self.choose(&[alias!("{p} {first}{v}")], c.start, c.nucleus + 1)? {
                return Ok(());
            }
        }
        let all = self.join(c.first, c.nucleus, "")?;
        if c.count() > 1
            && c.previous == 0
            && self.choose(&[alias!("{p} {all}{v}")], c.start, c.nucleus + 1)?
        {
            return Ok(());
        }
        let mut base = self.find_candidates(&[alias!("{last}{v}"), alias!("{last} {v}")])?;
        let mut base_start = c.nucleus - 1;
        if c.current() >= 2 {
            if let Some(found) =
                self.find_candidates(&[alias!("{all}{v}"), alias!("- {all}{v}")])?
            {
                base = Some(found);
                base_start = c.first;
            } else if let Some(found) = self.find_candidates(&[alias!("_{last}{v}")])? {
                base = Some(found);
            }
        }
        let Some(mut base) = base else {
            return self.missing(c.first, c.nucleus + 1);
        };
        let mut first_index = 0;
        let second = (c.first + 2).min(c.nucleus);
        let joined = self.join(c.first, second, "")?;
        let spaced = self.join(c.first, second, " ")?;
        let mut candidates = arrayvec::ArrayVec::<AliasText, 5>::new();
        if joined.as_str() != "tS" && joined.as_str() != "dZ" {
            candidates.push(alias!("{p} {joined}"));
        }
        candidates.push(alias!("{p}{spaced}"));
        if self.choose(&candidates, c.start, second)? {
            first_index = 1;
        } else if !self.choose(&[alias!("{p} {first}")], c.start, c.first + 1)? {
            let _ = self.choose(&[alias!("{p} -")], c.start, c.start + 1)?;
        }
        self.consonants(c, first_index, base_start, &mut base)?;
        self.add(base.0, base.1, base_start, c.nucleus + 1, base.2)
    }
    fn cluster(&mut self, c: &Context, index: usize) -> Result<Option<ClusterMatch>> {
        let begin = c.first + index;
        let pair = self.join(begin, begin + 2, " ")?;
        if c.current() >= 2 {
            let joined = self.join(begin, c.nucleus, "")?;
            if let Some(found) = self.find_candidates(&[joined])? {
                return Ok(Some((found.0, found.1, found.2, c.nucleus)));
            }
        }
        Ok(self
            .find_candidates(&[pair])?
            .map(|f| (f.0, f.1, f.2, begin + 2)))
    }
    fn consonants(
        &mut self,
        c: &Context,
        mut index: usize,
        base_start: usize,
        base: &mut (UnitId, OtoEntry, Substitutions),
    ) -> Result<()> {
        let mut last = base_start - c.first;
        while index < last {
            let begin = c.first + index;
            let one = self.cluster(c, index)?;
            if index + 1 < last {
                let _two = self.cluster(c, index + 1)?;
                let suffix = self.join(begin + 1, c.nucleus, "")?;
                let v = self.atoms[c.nucleus].symbol.text();
                if c.current() >= 2
                    && let Some(found) =
                        self.find_candidates(&[alias!("{suffix}{v}"), alias!("- {suffix}{v}")])?
                {
                    last = index;
                    *base = found;
                }
                if let Some(found) = one {
                    self.add(found.0, found.1, begin, found.3, found.2)?;
                    if found.3 == c.nucleus {
                        index += 1;
                    }
                } else {
                    let already = self.units.iter().any(|u| {
                        u.realizations.start as usize <= begin
                            && u.realizations.end as usize > begin
                    });
                    if !already {
                        let a = self.atoms[begin].symbol.text();
                        if let Some(found) = self.find_candidates(&[alias!("{a}")])? {
                            self.add(
                                found.0,
                                found.1,
                                begin,
                                begin + 1,
                                Substitutions::COVERAGE_REPAIR,
                            )?;
                        }
                    }
                }
            } else if let Some(found) = one {
                self.add(found.0, found.1, begin, found.3, found.2)?;
            } else {
                let a = self.atoms[begin].symbol.text();
                let _ = self.choose(&[alias!("{a}")], begin, begin + 1)?;
            }
            index += 1;
        }
        Ok(())
    }
}
