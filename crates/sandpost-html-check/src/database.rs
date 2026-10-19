//! Embedded caniemail.com support data and its support-code grammar.

use std::sync::OnceLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// The caniemail.com `api/data.json` snapshot, refreshed by `scripts/update-caniemail.mts`.
const EMBEDDED_DATA: &str = include_str!("../data/caniemail.json");

/// Versions tested for one client, in the publication order of the data (oldest first).
pub(crate) type VersionResults = IndexMap<String, String>;

/// The parts of the caniemail database used by the check.
#[derive(Debug, Deserialize)]
pub(crate) struct SupportDatabase {
    pub(crate) last_update_date: String,
    pub(crate) nicenames: DisplayNames,
    pub(crate) data: Vec<FeatureEntry>,
}

/// Human-readable names for client families and platforms, in the database's display order.
#[derive(Debug, Deserialize)]
pub(crate) struct DisplayNames {
    pub(crate) family: IndexMap<String, String>,
    pub(crate) platform: IndexMap<String, String>,
}

/// One tested HTML, CSS or image feature.
#[derive(Debug, Deserialize)]
pub(crate) struct FeatureEntry {
    pub(crate) slug: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    pub(crate) url: String,
    pub(crate) category: String,
    /// Support codes keyed by client family, then platform, then tested version.
    pub(crate) stats: IndexMap<String, IndexMap<String, VersionResults>>,
    #[serde(default)]
    pub(crate) notes_by_num: Option<IndexMap<String, String>>,
}

impl SupportDatabase {
    /**
     * Find a feature by its caniemail slug.
     */
    pub(crate) fn feature(&self, slug: &str) -> Option<&FeatureEntry> {
        self.data.iter().find(|feature| feature.slug == slug)
    }
}

/**
 * Return the embedded database, deserializing it on first use.
 *
 * The snapshot is validated by this crate's tests, so a failure here means the embedded file was
 * replaced without running them.
 */
pub(crate) fn support_database() -> &'static SupportDatabase {
    static DATABASE: OnceLock<SupportDatabase> = OnceLock::new();
    DATABASE.get_or_init(|| {
        serde_json::from_str(EMBEDDED_DATA).expect("embedded caniemail data is valid")
    })
}

/// How well one client version supports a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Yes,
    No,
    Partial,
    /// The feature was not conclusively tested for this version.
    Unknown,
}

/**
 * Parse a caniemail support code such as `y`, `n`, `a #1` or `y #2 #3`.
 *
 * The first word is the level (`a` means partial; anything unrecognized, including `u`, is unknown)
 * and every following `#<number>` word refers to a numbered note of the feature.
 */
pub(crate) fn parse_support(code: &str) -> (SupportLevel, Vec<u32>) {
    let mut words = code.split_whitespace();
    let level = match words.next() {
        Some("y") => SupportLevel::Yes,
        Some("n") => SupportLevel::No,
        Some("a") => SupportLevel::Partial,
        _ => SupportLevel::Unknown,
    };
    let notes = words
        .filter_map(|word| word.strip_prefix('#'))
        .filter_map(|number| number.parse().ok())
        .collect();
    (level, notes)
}
