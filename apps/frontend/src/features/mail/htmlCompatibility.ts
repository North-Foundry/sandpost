import type {
  CompatibilityFeature,
  CompatibilitySummary,
  EmailClient,
  HtmlCompatibilityReport,
  SupportLevel,
  WeightedScore,
} from '@/api/contracts'

/**
 * User-facing platform groups; caniemail platforms are folded into these.
 */
export const PLATFORM_GROUPS = ['desktop', 'webmail', 'ios', 'android'] as const

export type PlatformGroup = typeof PLATFORM_GROUPS[number]

/**
 * Whether the report counts only the main client families or every client in the data.
 */
export type ClientScope = 'main' | 'all'

/**
 * Where a feature belongs in the report under the current filter.
 */
export type FeatureSeverity = 'problem' | 'difference' | 'unscored' | 'supported'

/**
 * Which clients the report counts.
 */
export interface CompatibilityFilter {
  scope: ClientScope
  platforms: PlatformGroup[]

  /**
   * Also require support from every older tested version, not only the latest one.
   */
  olderVersions: boolean
}

/**
 * How one client handles one feature under the current filter.
 */
export interface FeatureClientResult {
  client: EmailClient
  support: SupportLevel

  /**
   * The versions whose result is `support`, oldest first.
   */
  versions: string[]
  notes: number[]
}

/**
 * Support counts of one feature across the selected clients.
 */
export type FeatureCounts = Record<SupportLevel, number>

/**
 * One piece of a caniemail markdown text, rendered without HTML injection.
 */
export type MarkdownSegment =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'link'; text: string; href: string }

export const DEFAULT_COMPATIBILITY_FILTER: CompatibilityFilter = { scope: 'main', platforms: [...PLATFORM_GROUPS], olderVersions: false }

/**
 * Severity order, worst first; also used to combine results of several versions.
 */
const SEVERITY: SupportLevel[] = ['no', 'partial', 'yes', 'unknown']

/**
 * Fold a caniemail platform into its user-facing group.
 */
export function platformGroup(platform: string): PlatformGroup {
  if (platform === 'ios') return 'ios'
  if (platform === 'android') return 'android'
  if (platform.includes('webmail') || platform === 'outlook-com') return 'webmail'
  return 'desktop'
}

/**
 * Return the clients of the report in the selected scope and platform groups.
 */
export function selectedClients(report: HtmlCompatibilityReport, filter: CompatibilityFilter): EmailClient[] {
  return report.clients.filter((client) => (filter.scope === 'all' || client.main) && filter.platforms.includes(platformGroup(client.platform)))
}

/**
 * Return the worse of two support levels; unknown never overrides a known result.
 */
function worse(first: SupportLevel, second: SupportLevel): SupportLevel {
  if (first === 'unknown') return second
  if (second === 'unknown') return first
  return SEVERITY.indexOf(first) <= SEVERITY.indexOf(second) ? first : second
}

/**
 * Resolve how each selected client handles a feature: the latest version only, or the worst
 * result across all versions when older versions are included. Clients without data are unknown.
 */
export function featureResults(feature: CompatibilityFeature, clients: EmailClient[], filter: CompatibilityFilter): FeatureClientResult[] {
  return clients.map((client) => {
    const results = feature.support.filter((result) => result.client === client.id && (filter.olderVersions || result.latest))
    const support = results.reduce<SupportLevel>((level, result) => worse(level, result.support), 'unknown')
    const matching = results.filter((result) => result.support === support)
    return {
      client,
      support,
      versions: matching.map((result) => result.version),
      notes: [...new Set(matching.flatMap((result) => result.notes))].sort((first, second) => first - second),
    }
  })
}

/**
 * Count the results of one feature by support level.
 */
export function featureCounts(results: FeatureClientResult[]): FeatureCounts {
  const counts: FeatureCounts = { yes: 0, no: 0, partial: 0, unknown: 0 }
  for (const result of results) counts[result.support]++
  return counts
}

/**
 * Client outcomes from best to worst; a client keeps the worst outcome of its feature results.
 */
const OUTCOMES = ['untested', 'identical', 'withDifferences', 'withProblems'] as const

type ClientOutcome = typeof OUTCOMES[number]

/**
 * Classify one feature result: an unsupported feature the layout or content depends on is a
 * problem, a missing graceful feature or partial support is a difference.
 */
function resultOutcome(support: SupportLevel, graceful: boolean): ClientOutcome {
  if (support === 'no') return graceful ? 'withDifferences' : 'withProblems'
  if (support === 'partial') return 'withDifferences'
  return support === 'yes' ? 'identical' : 'untested'
}

/**
 * Summarize how the selected clients render the message over the scored features: with problems,
 * with differences, identical, or untested when no scored feature has data for them.
 */
export function summarizeReport(report: HtmlCompatibilityReport, filter: CompatibilityFilter): CompatibilitySummary {
  const clients = selectedClients(report, filter)
  const outcomes = new Map<string, ClientOutcome>(clients.map((client) => [client.id, 'untested']))
  for (const feature of report.features.filter((candidate) => candidate.scored)) {
    for (const result of featureResults(feature, clients, filter)) {
      const outcome = resultOutcome(result.support, feature.graceful)
      const current = outcomes.get(result.client.id) ?? 'untested'
      if (OUTCOMES.indexOf(outcome) > OUTCOMES.indexOf(current)) outcomes.set(result.client.id, outcome)
    }
  }
  const summary: CompatibilitySummary = { clients: clients.length, identical: 0, withDifferences: 0, withProblems: 0, untested: 0 }
  for (const outcome of outcomes.values()) summary[outcome]++
  return summary
}

/**
 * Place a feature in the report: problems break the layout or content in some selected client,
 * differences only change how it looks, unscored features are listed apart, and the rest is fully
 * supported.
 */
export function featureSeverity(feature: CompatibilityFeature, counts: FeatureCounts): FeatureSeverity {
  if (!counts.no && !counts.partial) return 'supported'
  if (!feature.scored) return 'unscored'
  return counts.no && !feature.graceful ? 'problem' : 'difference'
}

/**
 * Return the fraction of body elements that use a feature, at most 1.
 */
export function featureWeight(feature: CompatibilityFeature, report: HtmlCompatibilityReport): number {
  return Math.min(feature.occurrences / Math.max(report.elementCount, 1), 1)
}

/**
 * Return the unsupported and partial shares of a feature, in percent of the clients with a known result.
 */
function featureShares(counts: FeatureCounts): { unsupported: number; partial: number } {
  const known = counts.yes + counts.no + counts.partial
  return known ? { unsupported: (100 * counts.no) / known, partial: (100 * counts.partial) / known } : { unsupported: 0, partial: 0 }
}

/**
 * Return a feature's impact: the percentage of clients without full support, weighted by its element share.
 */
export function featureImpact(counts: FeatureCounts, weight: number): number {
  const shares = featureShares(counts)
  return (shares.unsupported + shares.partial) * weight
}

/**
 * Compute the Mailpit-style weighted coverage over scored features under the filter: the worst
 * weighted unsupported and partial shares decide, and supported is the remainder.
 */
export function weightedScore(report: HtmlCompatibilityReport, filter: CompatibilityFilter): WeightedScore {
  const clients = selectedClients(report, filter)
  let unsupported = 0
  let partial = 0
  for (const feature of report.features.filter((candidate) => candidate.scored)) {
    const shares = featureShares(featureCounts(featureResults(feature, clients, filter)))
    const weight = featureWeight(feature, report)
    unsupported = Math.max(unsupported, shares.unsupported * weight)
    partial = Math.max(partial, shares.partial * weight)
  }
  return { supported: Math.max(100 - unsupported - partial, 0), partial, unsupported }
}

/**
 * Order features for display: scored before unscored, breaking before graceful, then by decreasing
 * impact, then by most unsupported clients, then by title.
 */
export function sortFeatures<Entry extends { feature: CompatibilityFeature; counts: FeatureCounts; impact: number }>(features: Entry[]): Entry[] {
  return [...features].sort((first, second) =>
    Number(second.feature.scored) - Number(first.feature.scored)
    || Number(first.feature.graceful) - Number(second.feature.graceful)
    || second.impact - first.impact
    || second.counts.no - first.counts.no
    || first.feature.title.localeCompare(second.feature.title))
}

/**
 * Split caniemail markdown into plain text, inline code and http(s) links; other markup stays text.
 */
export function markdownSegments(source: string): MarkdownSegment[] {
  const segments: MarkdownSegment[] = []
  const pattern = /`([^`]+)`|\[([^\]]+)\]\(([^)\s]+)\)/g
  let position = 0
  for (const match of source.matchAll(pattern)) {
    if (match.index > position) segments.push({ kind: 'text', text: source.slice(position, match.index) })
    if (match[1] !== undefined) segments.push({ kind: 'code', text: match[1] })
    else if (/^https?:\/\//i.test(match[3]!)) segments.push({ kind: 'link', text: match[2]!, href: match[3]! })
    else segments.push({ kind: 'text', text: match[2]! })
    position = match.index + match[0].length
  }
  if (position < source.length) segments.push({ kind: 'text', text: source.slice(position) })
  return segments
}

/**
 * Read a stored filter, falling back to the default for missing, malformed or unknown values; an
 * empty platform selection restores every platform.
 */
export function parseCompatibilityFilter(stored: string | null): CompatibilityFilter {
  try {
    const value = JSON.parse(stored ?? 'null') as Partial<CompatibilityFilter> | null
    const remembered = Array.isArray(value?.platforms) ? PLATFORM_GROUPS.filter((group) => value.platforms!.includes(group)) : []
    const platforms = remembered.length ? remembered : [...PLATFORM_GROUPS]
    return { scope: value?.scope === 'all' ? 'all' : 'main', platforms, olderVersions: value?.olderVersions === true }
  } catch {
    return { ...DEFAULT_COMPATIBILITY_FILTER, platforms: [...PLATFORM_GROUPS] }
  }
}
