//! Sequence sets (RFC 3501 section 9 `sequence-set`) for message numbers and UIDs.
//!
//! A range is unordered (`5:2` equals `2:5`) and `*` stands for the largest value in use, so
//! `100:*` includes the largest UID even when it is below 100.

/// One endpoint of a range: a number or `*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceNumber {
    /// A non-zero number.
    Value(u32),
    /// `*`: the largest number in use.
    Largest,
}

impl SequenceNumber {
    /// Resolve `*` to the largest value in use.
    fn resolve(self, largest: u32) -> u32 {
        match self {
            Self::Value(value) => value,
            Self::Largest => largest,
        }
    }
}

/// A single number or an unordered inclusive range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceRange {
    pub first: SequenceNumber,
    pub last: SequenceNumber,
}

/// A non-empty, comma-separated list of numbers and ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceSet {
    ranges: Vec<SequenceRange>,
}

/// Most ranges one sequence set may contain, bounding work for pathological input.
const MAXIMUM_RANGE_COUNT: usize = 4096;

impl SequenceSet {
    /// Parse `sequence-set`; zero, empty items, and malformed numbers are rejected.
    pub fn parse(text: &str) -> Option<Self> {
        let mut ranges = Vec::new();
        for item in text.split(',') {
            let range = match item.split_once(':') {
                Some((first, last)) => SequenceRange {
                    first: parse_number(first)?,
                    last: parse_number(last)?,
                },
                None => {
                    let number = parse_number(item)?;
                    SequenceRange {
                        first: number,
                        last: number,
                    }
                }
            };
            ranges.push(range);
            if ranges.len() > MAXIMUM_RANGE_COUNT {
                return None;
            }
        }
        Some(Self { ranges })
    }

    /// Build a set from explicit ranges; used by tests and response helpers.
    pub fn from_ranges(ranges: Vec<SequenceRange>) -> Self {
        Self { ranges }
    }

    /// Return the inclusive numeric ranges after resolving `*` to `largest`, ordered and merged.
    ///
    /// With `largest` zero, `*` resolves to zero, which never matches a valid number.
    pub fn resolve(&self, largest: u32) -> Vec<(u32, u32)> {
        let mut resolved: Vec<(u32, u32)> = self
            .ranges
            .iter()
            .map(|range| {
                let first = range.first.resolve(largest);
                let last = range.last.resolve(largest);
                (first.min(last), first.max(last))
            })
            .collect();
        resolved.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(resolved.len());
        for (first, last) in resolved {
            match merged.last_mut() {
                Some(previous) if first <= previous.1.saturating_add(1) => {
                    previous.1 = previous.1.max(last);
                }
                _ => merged.push((first, last)),
            }
        }
        merged
    }

    /// Report whether a value belongs to the set when `*` means `largest`.
    pub fn contains(&self, value: u32, largest: u32) -> bool {
        self.ranges.iter().any(|range| {
            let first = range.first.resolve(largest);
            let last = range.last.resolve(largest);
            (first.min(last)..=first.max(last)).contains(&value)
        })
    }

    /// Report whether any range uses `*`.
    pub fn uses_largest(&self) -> bool {
        self.ranges.iter().any(|range| {
            range.first == SequenceNumber::Largest || range.last == SequenceNumber::Largest
        })
    }

    /// Report whether the set names any explicit number greater than `largest`, ignoring `*`.
    ///
    /// Message sequence numbers above the mailbox size are a client error (RFC 3501 section
    /// 7.4.1 and 9 `seq-number`).
    pub fn exceeds(&self, largest: u32) -> bool {
        self.ranges.iter().any(|range| {
            [range.first, range.last]
                .into_iter()
                .any(|number| matches!(number, SequenceNumber::Value(value) if value > largest))
        })
    }
}

/// Parse one `seq-number`: `*` or a non-zero 32-bit number without sign or leading zero.
fn parse_number(text: &str) -> Option<SequenceNumber> {
    if text == "*" {
        return Some(SequenceNumber::Largest);
    }
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.starts_with('0') {
        return None;
    }
    let value: u32 = text.parse().ok()?;
    Some(SequenceNumber::Value(value))
}

/// Format UIDs compactly as a `sequence-set`, preserving their order and collapsing only
/// ascending runs of consecutive values (RFC 4315 COPYUID sets correspond by position).
pub fn format_ordered_set(values: &[u32]) -> String {
    let mut text = String::new();
    let mut index = 0;
    while index < values.len() {
        let start = values[index];
        let mut end = start;
        while index + 1 < values.len() && values[index + 1] == end.wrapping_add(1) {
            index += 1;
            end = values[index];
        }
        if !text.is_empty() {
            text.push(',');
        }
        if start == end {
            text.push_str(&start.to_string());
        } else {
            text.push_str(&format!("{start}:{end}"));
        }
        index += 1;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ranges are unordered, merged, and `*` resolves to the largest value.
    #[test]
    fn ranges_resolve_unordered_and_merge() {
        let set = SequenceSet::parse("5:2,7,8:*,1").unwrap();
        assert_eq!(set.resolve(10), [(1, 5), (7, 10)]);
        assert!(set.contains(3, 10) && !set.contains(6, 10) && set.contains(10, 10));
    }

    /// `n:*` includes the largest value even when it is below n (RFC 3501 section 6.4.8).
    #[test]
    fn star_range_includes_largest_below_start() {
        let set = SequenceSet::parse("100:*").unwrap();
        assert_eq!(set.resolve(50), [(50, 100)]);
        assert!(set.contains(50, 50));
    }

    /// Zero, empty items, signs, leading zeros, and overflow are malformed.
    #[test]
    fn malformed_sets_are_rejected() {
        for text in [
            "",
            "0",
            "1,,2",
            "1:",
            ":2",
            "+1",
            "01",
            "4294967296",
            "a",
            "1 2",
            "*:",
        ] {
            assert!(SequenceSet::parse(text).is_none(), "{text}");
        }
        assert!(SequenceSet::parse("4294967295").is_some());
    }

    /// Explicit numbers above the mailbox size are detected; `*` is not.
    #[test]
    fn explicit_numbers_above_largest_are_detected() {
        assert!(SequenceSet::parse("1:5").unwrap().exceeds(4));
        assert!(!SequenceSet::parse("1:*").unwrap().exceeds(4));
    }

    /// Ordered sets keep positional correspondence and collapse ascending runs.
    #[test]
    fn ordered_sets_collapse_ascending_runs() {
        assert_eq!(format_ordered_set(&[1, 2, 3, 7, 9, 10]), "1:3,7,9:10");
        assert_eq!(format_ordered_set(&[5, 4]), "5,4");
        assert_eq!(format_ordered_set(&[]), "");
    }
}
