import type {
  BulkLinkCheckResult,
  HtmlCompatibilityReport,
  LinkCheckErrorKind,
  LinkCheckReport,
  LinkOutcome,
  MessageDetail,
  MessageFlags,
  MessageSummary,
  SavedView,
  WebFolder,
} from './contracts'
import type {
  WireBulkLinkCheckResult,
  WireCompatibilityReport,
  WireLinkCheckReport,
  WireMailbox,
  WireMessageDetail,
  WireMessageFlags,
  WireMessageSummary,
  WireView,
  WireWebFolder,
} from './wire'

/**
 * Convert the Rust summary DTO to the stable frontend contract.
 */
export function normalizeSummary(
  representation: WireMessageSummary
): MessageSummary {
  return {
    id: String(representation.id),
    sequence: Number(representation.seq),
    subject: String(representation.subject ?? ''),
    from: addresses(representation.from),
    to: addresses(representation.to),
    envelopeTo: addresses(representation.envelope_to ?? representation.to),
    receivedAt: Number(representation.received_at) * 1000,
    size: Number(representation.size),
    attachmentCount: Number(representation.attachment_count),
    seen: Boolean(representation.flags?.seen ?? representation.seen),
    flags: normalizeFlags(representation.flags, representation.seen),
    ...(representation.preview !== undefined
      ? { preview: String(representation.preview) }
      : {}),
  }
}

/**
 * Normalize new flag summaries while deriving Seen from the legacy field for older servers.
 */
function normalizeFlags(
  flags: WireMessageFlags | undefined,
  legacySeen: boolean
): MessageFlags {
  return {
    seen: Boolean(flags?.seen ?? legacySeen),
    answered: Boolean(flags?.answered),
    flagged: Boolean(flags?.flagged),
    draft: Boolean(flags?.draft),
    deleted: Boolean(flags?.deleted),
    keywords: (flags?.keywords ?? []).map(String),
  }
}

/**
 * Flatten the Rust detail envelope and convert its Unix-second timestamp.
 */
export function normalizeDetail(
  representation: WireMessageDetail
): MessageDetail {
  const facts = representation.facts
  return {
    id: String(representation.id),
    subject: String(facts.subject ?? ''),
    from: addresses(facts.from),
    to: addresses(facts.to),
    cc: addresses(facts.cc),
    receivedAt: Number(facts.received_at) * 1000,
    text: String(facts.text ?? ''),
    html: String(facts.html ?? ''),
    htmlGenerated: Boolean(representation.html_generated),
    attachments: (representation.attachments ?? []).map(
      (attachment: Record<string, unknown>) => ({
        filename:
          attachment.filename == null ? null : String(attachment.filename),
        contentType: String(
          attachment.content_type ?? 'application/octet-stream'
        ),
        size: Number(attachment.size),
      })
    ),
    headers: facts.headers ?? {},
  }
}

/**
 * Convert a Rust saved view to the frontend contract.
 */
export function normalizeView(representation: WireView): SavedView {
  return {
    id: String(representation.identifier),
    ownerId:
      representation.owner_identifier == null
        ? null
        : String(representation.owner_identifier),
    name: String(representation.name),
    filter: String(representation.filter ?? ''),
    icon: representation.icon ?? null,
  }
}

/**
 * Encode a saved view as the Rust `View` body.
 */
export function viewWire(view: SavedView): WireView {
  return {
    identifier: view.id,
    owner_identifier: view.ownerId,
    name: view.name,
    filter: view.filter,
    icon: view.icon ?? null,
  }
}

/**
 * Convert a user-owned Web folder DTO and its Unix-second timestamps.
 */
export function normalizeWebFolder(representation: WireWebFolder): WebFolder {
  return {
    id: String(representation.identifier),
    ownerId: String(representation.owner_identifier),
    name: String(representation.name),
    icon: representation.icon ?? null,
    exposedAccounts: (representation.exposed_accounts ?? []).map(String),
    createdAt: Number(representation.created_at) * 1000,
    updatedAt: Number(representation.updated_at) * 1000,
  }
}

/**
 * Extract normalized MIME mailbox addresses while preserving their order.
 */
function addresses(value: WireMailbox[]): string[] {
  return value.map((mailbox) => mailbox.address)
}

/**
 * Convert the Rust compatibility report to the camel-case frontend contract.
 */
export function normalizeCompatibilityReport(
  representation: WireCompatibilityReport
): HtmlCompatibilityReport {
  return {
    htmlPresent: Boolean(representation.html_present),
    externalStylesheets: Number(representation.external_stylesheets ?? 0),
    elementCount: Number(representation.element_count ?? 0),
    data: {
      source: String(representation.data?.source ?? ''),
      url: String(representation.data?.url ?? ''),
      updated: String(representation.data?.updated ?? ''),
      license: String(representation.data?.license ?? ''),
    },
    clients: (representation.clients ?? []).map((client) => ({
      id: String(client.id),
      family: String(client.family),
      familyName: String(client.family_name),
      platform: String(client.platform),
      platformName: String(client.platform_name),
      main: Boolean(client.main),
    })),
    summary: {
      clients: Number(representation.summary?.clients ?? 0),
      identical: Number(representation.summary?.identical ?? 0),
      withDifferences: Number(representation.summary?.with_differences ?? 0),
      withProblems: Number(representation.summary?.with_problems ?? 0),
      untested: Number(representation.summary?.untested ?? 0),
    },
    weighted: {
      supported: Number(representation.weighted?.supported ?? 0),
      partial: Number(representation.weighted?.partial ?? 0),
      unsupported: Number(representation.weighted?.unsupported ?? 0),
    },
    features: (representation.features ?? []).map((feature) => ({
      slug: String(feature.slug),
      title: String(feature.title),
      category: String(feature.category),
      url: String(feature.url),
      description: String(feature.description ?? ''),
      notes: Object.fromEntries(Object.entries(feature.notes ?? {}).map(([number, note]) => [number, String(note)])),
      occurrences: Number(feature.occurrences ?? 0),
      examples: (feature.examples ?? []).map(String),
      selectorUnverified: Boolean(feature.selector_unverified),
      scored: feature.scored !== false,
      graceful: Boolean(feature.graceful),
      impact: Number(feature.impact ?? 0),
      support: (feature.support ?? []).map((result) => ({
        client: String(result.client),
        version: String(result.version),
        latest: Boolean(result.latest),
        support: result.support,
        notes: (result.notes ?? []).map(Number),
      })),
    })),
  }
}

const LINK_OUTCOMES: readonly LinkOutcome[] = ['healthy', 'broken', 'server_error', 'failed', 'inconclusive']
const LINK_CHECK_ERROR_KINDS: readonly LinkCheckErrorKind[] = [
  'name_resolution', 'connection', 'tls', 'timeout', 'protocol', 'too_many_redirects', 'redirect_loop', 'invalid_redirect', 'blocked_address',
]

/**
 * Read an optional Unix timestamp, keeping absence distinct from zero.
 */
function optionalTimestamp(value: number | null | undefined): number | null {
  return value === null || value === undefined ? null : Number(value)
}

/**
 * Convert the Rust link check state to the camel-case frontend contract.
 *
 * Unknown outcomes from a newer backend read as inconclusive and unknown error kinds as absent,
 * so the interface never presents an unrecognized verdict as healthy.
 */
export function normalizeLinkCheckReport(representation: WireLinkCheckReport): LinkCheckReport {
  return {
    messageId: String(representation.message_id),
    running: Boolean(representation.running),
    startedAt: optionalTimestamp(representation.started_at),
    checkedAt: optionalTimestamp(representation.checked_at),
    urls: (representation.urls ?? []).map(String),
    omittedLinks: Number(representation.omitted_links ?? 0),
    summary: {
      total: Number(representation.summary?.total ?? 0),
      healthy: Number(representation.summary?.healthy ?? 0),
      broken: Number(representation.summary?.broken ?? 0),
      serverError: Number(representation.summary?.server_error ?? 0),
      failed: Number(representation.summary?.failed ?? 0),
      inconclusive: Number(representation.summary?.inconclusive ?? 0),
    },
    links: (representation.links ?? []).map((link) => ({
      url: String(link.url),
      outcome: LINK_OUTCOMES.includes(link.outcome) ? link.outcome : 'inconclusive',
      statusCode: link.status_code === null || link.status_code === undefined ? null : Number(link.status_code),
      statusText: link.status_text ? String(link.status_text) : null,
      finalUrl: link.final_url ? String(link.final_url) : null,
      redirects: Number(link.redirects ?? 0),
      errorKind: LINK_CHECK_ERROR_KINDS.find((kind) => kind === link.error_kind) ?? null,
      errorMessage: link.error_message ? String(link.error_message) : null,
      checkedAt: Number(link.checked_at),
    })),
  }
}

/**
 * Convert the Rust bulk link check counts.
 */
export function normalizeBulkLinkCheck(representation: WireBulkLinkCheckResult): BulkLinkCheckResult {
  return {
    started: Number(representation.started ?? 0),
    alreadyRunning: Number(representation.already_running ?? 0),
    skipped: Number(representation.skipped ?? 0),
  }
}
