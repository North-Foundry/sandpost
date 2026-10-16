import type { MessageSummary, SandpostClient } from '@/api/contracts'

/**
 * Bulk deletion issues at most this many delete requests at once.
 */
export const DELETE_CONCURRENCY = 4

/**
 * Listing page size used to collect every readable message before deleting it.
 */
const COLLECT_PAGE_SIZE = 100

export interface BulkDeleteResult {
  deleted: Set<string>

  /**
   * Ids that could not be deleted, in request order.
   */
  failed: string[]

  /**
   * The first failure, for the message shown to the reader.
   */
  error: unknown
}

/**
 * Deletes every id with bounded concurrency, collecting failures instead of stopping at the first one.
 */
export async function deleteEach(client: SandpostClient, ids: readonly string[], onDeleted?: () => void): Promise<BulkDeleteResult> {
  const deleted = new Set<string>()
  const failed = new Set<string>()
  let error: unknown = null
  let next = 0

  /**
   * Claims ids from the shared cursor until this worker has no work left.
   */
  const deleteWorker = async () => {
    while (next < ids.length) {
      const id = ids[next++]!
      try {
        await client.messages.delete(id)
        deleted.add(id)
        onDeleted?.()
      } catch (reason) {
        failed.add(id)
        error ??= reason
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(DELETE_CONCURRENCY, ids.length) }, deleteWorker))
  return { deleted, failed: ids.filter((id) => failed.has(id)), error }
}

/**
 * Collects the id of every message the caller can read, newest first per mailbox: the unfiled Inbox, each manual
 * folder in `folderIds`, and the Trash, following each keyset cursor to the end. Mail arriving meanwhile is newer
 * than the cursor and is not included.
 */
export async function collectReadableIds(client: SandpostClient, folderIds: readonly string[] = []): Promise<string[]> {
  const ids = new Set<string>()
  const pagers: ((before: number | undefined) => Promise<MessageSummary[]>)[] = [
    (before) => client.messages.list({ before, limit: COLLECT_PAGE_SIZE }),
    ...folderIds.map((id) => (before: number | undefined) => client.folders.listMessages(id, { before, limit: COLLECT_PAGE_SIZE })),
    (before) => client.trash.listMessages({ before, limit: COLLECT_PAGE_SIZE }),
  ]
  for (const listPage of pagers) {
    let before: number | undefined
    for (; ;) {
      const page = await listPage(before)
      if (!page.length) break
      for (const message of page) ids.add(message.id)
      before = page.reduce((oldest, message) => Math.min(oldest, message.sequence), Infinity)
    }
  }
  return [...ids]
}
