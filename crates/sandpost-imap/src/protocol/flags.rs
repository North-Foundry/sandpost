//! Message flags: the RFC 3501 system flags and client-defined keywords.
//!
//! `\Recent` is session state (RFC 3501 section 2.3.2) and therefore never part of [`Flags`]; the
//! session tracks it separately. Keywords are compared ASCII-case-insensitively and keep the
//! spelling of their first use.

/// A persistent system flag defined by RFC 3501 section 2.3.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemFlag {
    /// `\Seen`: the message has been read.
    Seen,
    /// `\Answered`: the message has been answered.
    Answered,
    /// `\Flagged`: the message is flagged for urgent or special attention.
    Flagged,
    /// `\Deleted`: the message is marked for removal by a later EXPUNGE.
    Deleted,
    /// `\Draft`: the message has not completed composition.
    Draft,
}

impl SystemFlag {
    /// Every persistent system flag in the order the server announces them.
    pub const ALL: [Self; 5] = [
        Self::Answered,
        Self::Flagged,
        Self::Deleted,
        Self::Seen,
        Self::Draft,
    ];

    /// The flag's protocol spelling, including its leading backslash.
    pub fn name(self) -> &'static str {
        match self {
            Self::Seen => "\\Seen",
            Self::Answered => "\\Answered",
            Self::Flagged => "\\Flagged",
            Self::Deleted => "\\Deleted",
            Self::Draft => "\\Draft",
        }
    }

    /// The flag's bit in the compact [`Flags::system_bits`] representation.
    pub fn bit(self) -> u8 {
        match self {
            Self::Seen => 1,
            Self::Answered => 2,
            Self::Flagged => 4,
            Self::Deleted => 8,
            Self::Draft => 16,
        }
    }

    /// Recognize a system flag name case-insensitively, without `\Recent`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|flag| flag.name().eq_ignore_ascii_case(name))
    }
}

/// Longest keyword the server stores; longer keywords are refused as invalid arguments.
pub const MAXIMUM_KEYWORD_LENGTH: usize = 64;
/// Most keywords one message may carry; further keywords are refused.
pub const MAXIMUM_KEYWORDS_PER_MESSAGE: usize = 32;

/// The persistent flags of one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    system: u8,
    keywords: Vec<String>,
}

impl Flags {
    /// Flags with nothing set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild flags from their compact system bits and keywords, discarding unknown bits,
    /// invalid keywords, and case-insensitive duplicates.
    pub fn from_parts(system_bits: u8, keywords: impl IntoIterator<Item = String>) -> Self {
        let mut flags = Self {
            system: system_bits & 0b1_1111,
            keywords: Vec::new(),
        };
        for keyword in keywords {
            if is_valid_keyword(&keyword) {
                flags.insert_keyword(&keyword);
            }
        }
        flags
    }

    /// The set system flags as bits of [`SystemFlag::bit`].
    pub fn system_bits(&self) -> u8 {
        self.system
    }

    /// Report whether a system flag is set.
    pub fn contains(&self, flag: SystemFlag) -> bool {
        self.system & flag.bit() != 0
    }

    /// Set a system flag.
    pub fn insert(&mut self, flag: SystemFlag) {
        self.system |= flag.bit();
    }

    /// Clear a system flag.
    pub fn remove(&mut self, flag: SystemFlag) {
        self.system &= !flag.bit();
    }

    /// The keywords in ASCII-case-insensitive order.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// Report whether a keyword is set, comparing ASCII-case-insensitively.
    pub fn has_keyword(&self, keyword: &str) -> bool {
        self.keywords
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(keyword))
    }

    /// Add a keyword unless an equal one (ignoring ASCII case) is already present.
    pub fn insert_keyword(&mut self, keyword: &str) {
        if self.has_keyword(keyword) {
            return;
        }
        let position = self
            .keywords
            .partition_point(|existing| keyword_order(existing) < keyword_order(keyword));
        self.keywords.insert(position, keyword.to_owned());
    }

    /// Remove a keyword, comparing ASCII-case-insensitively.
    pub fn remove_keyword(&mut self, keyword: &str) {
        self.keywords
            .retain(|existing| !existing.eq_ignore_ascii_case(keyword));
    }

    /// Return the flags that result from applying a STORE change.
    pub fn apply(&self, change: &FlagChange) -> Self {
        match change.operation {
            FlagOperation::Replace => change.flags.clone(),
            FlagOperation::Add => {
                let mut result = self.clone();
                result.system |= change.flags.system;
                for keyword in &change.flags.keywords {
                    result.insert_keyword(keyword);
                }
                result
            }
            FlagOperation::Remove => {
                let mut result = self.clone();
                result.system &= !change.flags.system;
                for keyword in &change.flags.keywords {
                    result.remove_keyword(keyword);
                }
                result
            }
        }
    }

    /// Report whether no flag is set.
    pub fn is_empty(&self) -> bool {
        self.system == 0 && self.keywords.is_empty()
    }
}

/// Sort key that orders keywords ASCII-case-insensitively.
fn keyword_order(keyword: &str) -> String {
    keyword.to_ascii_lowercase()
}

/// Report whether a keyword is an IMAP atom that does not start with a backslash.
///
/// Keywords are `flag-keyword = atom` (RFC 3501 section 9) and bounded by
/// [`MAXIMUM_KEYWORD_LENGTH`].
pub fn is_valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty()
        && keyword.len() <= MAXIMUM_KEYWORD_LENGTH
        && keyword.bytes().all(crate::protocol::syntax::is_atom_byte)
}

/// How a STORE command changes the flags of each message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagOperation {
    /// `FLAGS`: replace the flags.
    Replace,
    /// `+FLAGS`: add the flags.
    Add,
    /// `-FLAGS`: remove the flags.
    Remove,
}

/// A STORE change: the operation and the flags it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagChange {
    pub operation: FlagOperation,
    pub flags: Flags,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// System flags parse case-insensitively and `\Recent` is not a storable flag.
    #[test]
    fn system_flags_are_recognized_without_recent() {
        assert_eq!(SystemFlag::from_name("\\SEEN"), Some(SystemFlag::Seen));
        assert_eq!(SystemFlag::from_name("\\draft"), Some(SystemFlag::Draft));
        assert_eq!(SystemFlag::from_name("\\Recent"), None);
        assert_eq!(SystemFlag::from_name("Seen"), None);
    }

    /// Keywords deduplicate ignoring case, keep their first spelling, and stay ordered.
    #[test]
    fn keywords_are_case_insensitive_sets() {
        let mut flags = Flags::new();
        flags.insert_keyword("$Forwarded");
        flags.insert_keyword("$forwarded");
        flags.insert_keyword("$Junk");
        flags.insert_keyword("$label1");
        assert_eq!(flags.keywords(), ["$Forwarded", "$Junk", "$label1"]);
        assert!(flags.has_keyword("$JUNK"));
        flags.remove_keyword("$JUNK");
        assert_eq!(flags.keywords(), ["$Forwarded", "$label1"]);
    }

    /// Replace, add, and remove follow RFC 3501 STORE semantics.
    #[test]
    fn store_operations_replace_add_and_remove() {
        let mut current = Flags::new();
        current.insert(SystemFlag::Seen);
        current.insert_keyword("Work");
        let mut change_flags = Flags::new();
        change_flags.insert(SystemFlag::Flagged);
        change_flags.insert_keyword("work");
        let added = current.apply(&FlagChange {
            operation: FlagOperation::Add,
            flags: change_flags.clone(),
        });
        assert!(added.contains(SystemFlag::Seen) && added.contains(SystemFlag::Flagged));
        assert_eq!(added.keywords(), ["Work"]);
        let removed = added.apply(&FlagChange {
            operation: FlagOperation::Remove,
            flags: change_flags.clone(),
        });
        assert!(removed.contains(SystemFlag::Seen) && !removed.contains(SystemFlag::Flagged));
        assert!(removed.keywords().is_empty());
        let replaced = removed.apply(&FlagChange {
            operation: FlagOperation::Replace,
            flags: change_flags,
        });
        assert!(!replaced.contains(SystemFlag::Seen));
        assert!(replaced.contains(SystemFlag::Flagged));
    }

    /// Keywords must be bounded atoms without a leading backslash.
    #[test]
    fn keyword_validation_follows_atom_syntax() {
        assert!(is_valid_keyword("$MDNSent"));
        assert!(!is_valid_keyword("\\Seen"));
        assert!(!is_valid_keyword("two words"));
        assert!(!is_valid_keyword("brace{"));
        assert!(!is_valid_keyword(""));
        assert!(!is_valid_keyword(&"k".repeat(MAXIMUM_KEYWORD_LENGTH + 1)));
        let rebuilt = Flags::from_parts(0xff, ["ok".to_owned(), "not ok".to_owned()]);
        assert_eq!(rebuilt.system_bits(), 0b1_1111);
        assert_eq!(rebuilt.keywords(), ["ok"]);
    }
}
