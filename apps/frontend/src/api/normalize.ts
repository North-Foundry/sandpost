import type {
  MessageDetail,
  MessageFlags,
  MessageSummary,
  SavedView,
  WebFolder,
} from './contracts'
import type {
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
