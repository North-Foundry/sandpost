//! The compatibility report and its per-client scoring.

use std::cmp::Ordering;
use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};
use serde::Serialize;

use crate::database::{SupportDatabase, SupportLevel, parse_support};
use crate::detection::Detection;
use crate::features::{GRACEFUL_FEATURES, MAIN_CLIENT_FAMILIES, UNSCORED_FEATURES};

/// The result of checking one HTML body against the caniemail data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompatibilityReport {
    /// Whether the message has an HTML part; without one, the other lists are empty.
    pub html_present: bool,
    /// Linked stylesheets, which are never downloaded and therefore not analysed.
    pub external_stylesheets: usize,
    /// Elements inside `<body>`, the denominator of each feature's weight.
    pub element_count: usize,
    pub data: DataSource,
    /// Every client known to the data, in display order.
    pub clients: Vec<EmailClient>,
    /// Client outcomes over scored features, using each client's most recently tested version.
    pub summary: CompatibilitySummary,
    /// Mailpit-style weighted coverage over scored features, using latest versions.
    pub weighted: WeightedScore,
    /// Detected features: scored ones first, by decreasing impact.
    pub features: Vec<FeatureReport>,
}

/// Weighted coverage in percent.
///
/// Each feature's share of clients without full support is weighted by the fraction of elements
/// that use it (at most 1); the single worst weighted feature decides, so one widely used
/// unsupported feature matters more than a rare one.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WeightedScore {
    pub supported: f64,
    pub partial: f64,
    pub unsupported: f64,
}

/// Where the support data comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataSource {
    pub source: String,
    pub url: String,
    /// The `last_update_date` of the embedded snapshot.
    pub updated: String,
    pub license: String,
}

/// A client family on one platform, such as Gmail on Android.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmailClient {
    /// `<family>.<platform>`, as referenced by [`ClientSupport::client`].
    pub id: String,
    pub family: String,
    pub family_name: String,
    pub platform: String,
    pub platform_name: String,
    /// Whether the client belongs to the families most recipients use.
    pub main: bool,
}

/// How clients render the message across the scored features.
///
/// A client has problems when it does not support a feature the layout or content depends on;
/// it renders with differences when it only lacks graceful features or supports some features
/// partially; it is identical when every known result is full support.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompatibilitySummary {
    pub clients: usize,
    pub identical: usize,
    pub with_differences: usize,
    pub with_problems: usize,
    /// Clients for which none of the detected features has a known result.
    pub untested: usize,
}

/// One detected feature with its support results.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeatureReport {
    pub slug: String,
    pub title: String,
    pub category: String,
    pub url: String,
    /// Markdown text from caniemail.
    pub description: String,
    /// Markdown notes referenced by [`ClientSupport::notes`], keyed by number.
    pub notes: IndexMap<String, String>,
    pub occurrences: usize,
    pub examples: Vec<String>,
    /// True when every occurrence comes from a selector that could not be checked against the document.
    pub selector_unverified: bool,
    /// False for document-structure features that clients remove without breaking the message;
    /// they are listed but excluded from the summary and weighted score.
    pub scored: bool,
    /// True when clients without support only lose an enhancement, accessibility metadata or a
    /// cosmetic detail; such features cause differences, never problems.
    pub graceful: bool,
    /// Percentage points of clients without full support, weighted by the feature's element share.
    pub impact: f64,
    /// Results for every tested client version, oldest version first within a client.
    pub support: Vec<ClientSupport>,
}

/// The support of one client version for one feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientSupport {
    pub client: String,
    pub version: String,
    /// Whether this is the most recently tested version of the client for this feature.
    pub latest: bool,
    pub support: SupportLevel,
    pub notes: Vec<u32>,
}

/// The outcome of one client across all detected features.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ClientOutcome {
    Untested,
    Identical,
    WithDifferences,
    WithProblems,
}

impl ClientOutcome {
    /**
     * Combine the outcome so far with one more feature result; the worst known outcome wins.
     */
    fn combine(self, level: SupportLevel, graceful: bool) -> Self {
        let outcome = match level {
            SupportLevel::No if !graceful => ClientOutcome::WithProblems,
            SupportLevel::No | SupportLevel::Partial => ClientOutcome::WithDifferences,
            SupportLevel::Yes => ClientOutcome::Identical,
            SupportLevel::Unknown => ClientOutcome::Untested,
        };
        self.max(outcome)
    }
}

impl CompatibilityReport {
    /**
     * Build the report for a message without an HTML part.
     */
    pub fn without_html() -> Self {
        Self {
            html_present: false,
            external_stylesheets: 0,
            element_count: 0,
            data: data_source(crate::database::support_database()),
            clients: Vec::new(),
            summary: CompatibilitySummary::default(),
            weighted: WeightedScore::default(),
            features: Vec::new(),
        }
    }
}

/**
 * Describe the embedded data snapshot.
 */
fn data_source(database: &SupportDatabase) -> DataSource {
    DataSource {
        source: "caniemail.com".to_owned(),
        url: "https://www.caniemail.com".to_owned(),
        updated: database.last_update_date.clone(),
        license: "MIT".to_owned(),
    }
}

/// Latest-version shares of one feature, in percent of the clients with a known result.
struct LatestShares {
    unsupported: f64,
    partial: f64,
}

/**
 * Build the report for the features detected in an HTML body.
 */
pub(crate) fn build_report(
    detection: &Detection,
    database: &SupportDatabase,
) -> CompatibilityReport {
    let clients = all_clients(database);
    let element_count = detection.element_count.max(1);
    let mut features: Vec<(FeatureReport, LatestShares)> = Vec::new();
    for (slug, finding) in detection.findings.iter() {
        let Some(entry) = database.feature(slug) else {
            continue;
        };
        let mut support = Vec::new();
        for (family, platforms) in &entry.stats {
            for (platform, versions) in platforms {
                for (index, (version, code)) in versions.iter().enumerate() {
                    let (level, notes) = parse_support(code);
                    support.push(ClientSupport {
                        client: client_identifier(family, platform),
                        version: version.clone(),
                        latest: index + 1 == versions.len(),
                        support: level,
                        notes,
                    });
                }
            }
        }
        let shares = latest_shares(&support);
        let weight = (finding.occurrences as f64 / element_count as f64).min(1.0);
        let report = FeatureReport {
            slug: entry.slug.clone(),
            title: entry.title.clone(),
            category: entry.category.clone(),
            url: entry.url.clone(),
            description: entry.description.clone().unwrap_or_default(),
            notes: entry.notes_by_num.clone().unwrap_or_default(),
            occurrences: finding.occurrences,
            examples: finding.examples.clone(),
            selector_unverified: finding.unverified_occurrences == finding.occurrences,
            scored: !UNSCORED_FEATURES.contains(&slug),
            graceful: GRACEFUL_FEATURES.contains(&slug)
                || finding.graceful_occurrences == finding.occurrences,
            impact: rounded((shares.unsupported + shares.partial) * weight),
            support,
        };
        features.push((report, shares));
    }
    features.sort_by(|(first, first_shares), (second, second_shares)| {
        second
            .scored
            .cmp(&first.scored)
            .then(first.graceful.cmp(&second.graceful))
            .then(
                second
                    .impact
                    .partial_cmp(&first.impact)
                    .unwrap_or(Ordering::Equal),
            )
            .then(
                second_shares
                    .unsupported
                    .partial_cmp(&first_shares.unsupported)
                    .unwrap_or(Ordering::Equal),
            )
            .then_with(|| first.title.cmp(&second.title))
    });
    let weighted = weighted_score(&features, element_count);
    let features: Vec<FeatureReport> = features.into_iter().map(|(report, _)| report).collect();
    CompatibilityReport {
        html_present: true,
        external_stylesheets: detection.external_stylesheets,
        element_count: detection.element_count,
        data: data_source(database),
        summary: summarize(&clients, &features),
        weighted,
        clients,
        features,
    }
}

/**
 * Compute a feature's unsupported and partial shares among latest versions with a known result.
 */
fn latest_shares(support: &[ClientSupport]) -> LatestShares {
    let latest: Vec<SupportLevel> = support
        .iter()
        .filter(|result| result.latest && result.support != SupportLevel::Unknown)
        .map(|result| result.support)
        .collect();
    if latest.is_empty() {
        return LatestShares {
            unsupported: 0.0,
            partial: 0.0,
        };
    }
    let share = |level: SupportLevel| {
        100.0
            * latest
                .iter()
                .filter(|candidate| **candidate == level)
                .count() as f64
            / latest.len() as f64
    };
    LatestShares {
        unsupported: share(SupportLevel::No),
        partial: share(SupportLevel::Partial),
    }
}

/**
 * Compute the weighted coverage from the worst weighted shares of the scored features.
 */
fn weighted_score(
    features: &[(FeatureReport, LatestShares)],
    element_count: usize,
) -> WeightedScore {
    let mut unsupported: f64 = 0.0;
    let mut partial: f64 = 0.0;
    for (feature, shares) in features.iter().filter(|(feature, _)| feature.scored) {
        let weight = (feature.occurrences as f64 / element_count as f64).min(1.0);
        unsupported = unsupported.max(shares.unsupported * weight);
        partial = partial.max(shares.partial * weight);
    }
    WeightedScore {
        supported: rounded((100.0 - unsupported - partial).max(0.0)),
        partial: rounded(partial),
        unsupported: rounded(unsupported),
    }
}

/**
 * Round a percentage to one decimal place.
 */
fn rounded(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/**
 * Classify every client over the scored features using its most recently tested version.
 */
fn summarize(clients: &[EmailClient], features: &[FeatureReport]) -> CompatibilitySummary {
    let mut outcomes: HashMap<&str, ClientOutcome> = clients
        .iter()
        .map(|client| (client.id.as_str(), ClientOutcome::Untested))
        .collect();
    for feature in features.iter().filter(|feature| feature.scored) {
        for result in feature.support.iter().filter(|result| result.latest) {
            if let Some(outcome) = outcomes.get_mut(result.client.as_str()) {
                *outcome = outcome.combine(result.support, feature.graceful);
            }
        }
    }
    let mut summary = CompatibilitySummary {
        clients: clients.len(),
        ..CompatibilitySummary::default()
    };
    for outcome in outcomes.values() {
        match outcome {
            ClientOutcome::Untested => summary.untested += 1,
            ClientOutcome::Identical => summary.identical += 1,
            ClientOutcome::WithDifferences => summary.with_differences += 1,
            ClientOutcome::WithProblems => summary.with_problems += 1,
        }
    }
    summary
}

/**
 * List every family and platform pair that appears in the data, ordered by the data's display names.
 */
fn all_clients(database: &SupportDatabase) -> Vec<EmailClient> {
    let mut pairs: IndexSet<(&str, &str)> = IndexSet::new();
    for feature in &database.data {
        for (family, platforms) in &feature.stats {
            for platform in platforms.keys() {
                pairs.insert((family.as_str(), platform.as_str()));
            }
        }
    }
    let position =
        |names: &IndexMap<String, String>, key: &str| names.get_index_of(key).unwrap_or(usize::MAX);
    let mut pairs: Vec<_> = pairs.into_iter().collect();
    pairs.sort_by_key(|(family, platform)| {
        (
            position(&database.nicenames.family, family),
            position(&database.nicenames.platform, platform),
        )
    });
    pairs
        .into_iter()
        .map(|(family, platform)| EmailClient {
            id: client_identifier(family, platform),
            family: family.to_owned(),
            family_name: display_name(&database.nicenames.family, family),
            platform: platform.to_owned(),
            platform_name: display_name(&database.nicenames.platform, platform),
            main: MAIN_CLIENT_FAMILIES.contains(&family),
        })
        .collect()
}

/**
 * Return the display name of a family or platform, falling back to its key.
 */
fn display_name(names: &IndexMap<String, String>, key: &str) -> String {
    names.get(key).cloned().unwrap_or_else(|| key.to_owned())
}

/**
 * Build the client identifier used to link support results to clients.
 */
fn client_identifier(family: &str, platform: &str) -> String {
    format!("{family}.{platform}")
}
