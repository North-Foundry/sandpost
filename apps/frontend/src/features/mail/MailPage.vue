<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import type { MessageDetail, MessageFlagOperation, MessageFlags, MessageSummary } from '@/api/contracts';
import { useSandpostClient } from '@/runtime/client';
import { cn } from '@/lib/utils';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Button } from '@/components/ui/button';
import { DialogFooter } from '@/components/ui/dialog';
import { Spinner } from '@/components/ui/spinner';
import BusyDialog from '@/components/BusyDialog.vue';
import MessageList from '@/features/mail/MessageList.vue';
import MessageDetailPane from '@/features/mail/MessageDetail.vue';
import MessageSelectionPanel from '@/features/mail/MessageSelectionPanel.vue';
import { deleteEach } from '@/features/mail/bulkDelete';
import { orderedSelection } from '@/features/mail/messageSelection';
import { interpretSearch } from '@/features/mail/searchSyntax';
import { DEFAULT_INBOX, useWorkspace } from '@/features/shell/workspace';

const PAGE_SIZE = 50;

/**
 * The server places or trashes at most this many messages per request.
 */
const MESSAGE_BATCH_SIZE = 256;

/**
 * The confirmation lists this many subjects before summarizing the rest.
 */
const DELETE_SUBJECT_PREVIEW = 3;
const client = useSandpostClient();
const { t, locale } = useI18n()
const workspace = useWorkspace();
const route = useRoute();
const router = useRouter();
const messages = ref<MessageSummary[]>([]);
const unreadOnly = ref(false);

/**
 * Visible read indicators follow authoritative server summaries.
 */
const readMessageIds = computed(() => new Set(messages.value.filter((message) => message.seen).map((message) => message.id)));
const loading = ref(false);
const refreshing = ref(false);
const loadingOlder = ref(false);
const hasOlder = ref(false);
const listError = ref<string | null>(null);
const detail = ref<MessageDetail | null>(null);
const detailLoading = ref(false);
const detailSeen = ref(false);
const pendingSeenRequests = ref(0);
const pendingFlagRequests = ref(0);
const copying = ref(false);
const detailError = ref<string | null>(null);
const failedDownload = ref<'raw' | number | null>(null);
const downloadingAttachment = ref<number | null>(null);

/**
 * Messages chosen with Shift/Cmd-click for a bulk action; empty while only the open message is active.
 */
const selection = ref<Set<string>>(new Set());
const deleteOpen = ref(false);
const deleting = ref(false);
const deleteError = ref('');

/**
 * The messages awaiting delete confirmation, in list order.
 */
const deleteIds = ref<string[]>([]);
const deletedCount = ref(0);

/**
 * Messages are moving to Trash; that needs no confirmation, so only repeated requests are held back.
 */
const trashing = ref(false);
const emptyTrashOpen = ref(false);
const emptyingTrash = ref(false);
const emptyTrashError = ref('');

/**
 * Trashed messages known when Empty Trash was requested; null when older pages leave the total unknown.
 */
const emptyTrashCount = ref<number | null>(null);
const onMailRoute = computed(() => route.name === 'mail' || route.name === 'mail-message');
const selectedId = computed(() => onMailRoute.value && typeof route.params.id === 'string' ? route.params.id : null);
const selectedMessage = computed(() => messages.value.find((message) => message.id === selectedId.value) ?? null);

/**
 * The folder is part of the URL so every mail view is restorable and linkable.
 */
const folderId = computed(() => typeof route.query.folder === 'string' ? route.query.folder : DEFAULT_INBOX.id);
const appliedSearch = computed(() => typeof route.query.q === 'string' ? route.query.q.trim() : '');
const folder = computed(() => workspace.findFolder(folderId.value));

/**
 * A non-default folder named in the URL cannot be read until the sidebar's folder list arrives.
 */
const folderPending = computed(() => !folder.value && !workspace.foldersLoaded.value);

/**
 * Inside Trash, Delete is permanent and confirmed; everywhere else, search results included, it moves mail to Trash.
 */
const inTrash = computed(() => !appliedSearch.value && folder.value?.kind === 'trash');

/**
 * Viewers only read mail, so they are not offered Empty Trash, as they are not offered Delete all emails.
 */
const canDeleteMail = computed(() => workspace.user.value !== null && workspace.user.value.role !== 'viewer');
const visibleMessages = computed(() => messages.value.filter((message) => !unreadOnly.value || !readMessageIds.value.has(message.id)));
const multipleSelected = computed(() => selection.value.size > 1);
const selectedInOrder = computed(() => orderedSelection(visibleMessages.value.map((message) => message.id), selection.value));
const selectedFlags = computed(() => selectedInOrder.value.flatMap((id) => {
  const message = messages.value.find((candidate) => candidate.id === id);
  return message ? [message.flags ?? { seen: message.seen, answered: false, flagged: false, draft: false, deleted: false, keywords: [] }] : [];
}));
const deleteTitle = computed(() => deleteIds.value.length === 1 ? t('mail.delete.oneTitle') : t('mail.delete.manyTitle', { count: deleteIds.value.length }));
const deleteDescription = computed(() => deleteIds.value.length === 1
  ? t('mail.delete.oneDescription')
  : t('mail.delete.manyDescription'));
const emptyTrashDescription = computed(() => {
  const count = emptyTrashCount.value;
  const subject = count === null ? t('mail.delete.allEmails') : count === 1 ? t('mail.delete.theEmail') : t('mail.delete.allCount', { count: new Intl.NumberFormat(locale.value).format(count) });
  return t('mail.delete.emptyDescription', { subject });
});

/**
 * Subjects shown in the confirmation, so the reader can check what a bulk delete covers.
 */
const deleteSubjects = computed(() => deleteIds.value.slice(0, DELETE_SUBJECT_PREVIEW).map((id) =>
  (messages.value.find((message) => message.id === id)?.subject ?? (detail.value?.id === id ? detail.value.subject : '')) || t('mail.detail.noSubject')));
const listKey = computed(() => `${folderId.value}\n${appliedSearch.value}\n${folder.value?.kind ?? ''}`);

let unsubscribeEvents: (() => void) | null = null;
let stopped = false;
let detailRequest = 0;
let refreshRequest: Promise<void> | null = null;
let refreshAgain = false;
let listGeneration = 0;

/**
 * Reads a page from the active folder or the applied search, which spans all readable mail.
 */
function fetchMessages(before?: number): Promise<MessageSummary[]> {
  if (appliedSearch.value) {
    const interpreted = interpretSearch(appliedSearch.value);
    if (interpreted.kind === 'invalid') return Promise.reject(new Error(interpreted.message));
    if (!('query' in interpreted)) return Promise.resolve([]);
    return client.messages.list({ query: interpreted.query, before, limit: PAGE_SIZE });
  }
  const current = folder.value;
  if (!current) return Promise.resolve([]);
  if (current.kind === 'inbox') return client.messages.list({ before, limit: PAGE_SIZE });
  if (current.kind === 'trash') return client.trash.listMessages({ before, limit: PAGE_SIZE });
  return current.kind === 'smart'
    ? client.messages.list({ viewId: current.id, before, limit: PAGE_SIZE })
    : current.kind === 'folder'
      ? client.folders.listMessages(current.id, { before, limit: PAGE_SIZE })
      : client.messages.list({ inboxId: current.id, before, limit: PAGE_SIZE });
}

/**
 * Reloads the newest page and coalesces event updates while a fetch is pending.
 */
async function refreshMessages(): Promise<void> {
  if (folderPending.value) { loading.value = true; return; }
  if (refreshRequest) { refreshAgain = true; return refreshRequest; }

  /**
   * Runs one or more coalesced refresh passes until no event arrived mid-fetch.
   */
  const request = (async () => {
    do {
      refreshAgain = false;
      const generation = ++listGeneration;
      loading.value = messages.value.length === 0;
      refreshing.value = !loading.value;
      listError.value = null;
      try {
        const newest = await fetchMessages();
        if (stopped) return;
        if (generation !== listGeneration) continue;
        messages.value = newest;
        hasOlder.value = newest.length === PAGE_SIZE;
      } catch (error) {
        if (generation === listGeneration) listError.value = error instanceof Error ? error.message : t('mail.error.inboxUnavailable');
      } finally {
        if (generation === listGeneration) { loading.value = false; refreshing.value = false; }
      }
    } while (refreshAgain && !stopped);
  })();
  refreshRequest = request;
  await request;
  if (refreshRequest === request) refreshRequest = null;
}

/**
 * Move messages in API-sized batches and refresh the current mailbox; moving out of Trash restores them.
 */
async function moveMessages(identifiers: string[], folderIdentifier: string | null): Promise<void> {
  try {
    for (let offset = 0; offset < identifiers.length; offset += MESSAGE_BATCH_SIZE) {
      await client.messages.move(identifiers.slice(offset, offset + MESSAGE_BATCH_SIZE), folderIdentifier)
    }
    selection.value = new Set()
    if (selectedId.value && identifiers.includes(selectedId.value)) await router.push({ name: 'mail', query: route.query })
    workspace.refreshInboxUnread()
    await refreshMessages()
  } catch (failure) {
    listError.value = failure instanceof Error ? failure.message : t('mail.error.moveFailed')
  }
}

/**
 * Copy messages into an additional primary folder, retaining failed batches in the selection.
 */
async function copyMessages(identifiers: string[], folderIdentifier: string | null): Promise<void> {
  if (!identifiers.length || copying.value) return;
  copying.value = true;
  const copied = new Set<string>();
  let failure: unknown = null;
  try {
    for (let offset = 0; offset < identifiers.length; offset += MESSAGE_BATCH_SIZE) {
      const batch = identifiers.slice(offset, offset + MESSAGE_BATCH_SIZE);
      await client.messages.copy(batch, folderIdentifier);
      for (const id of batch) copied.add(id);
    }
  } catch (error) { failure = error; }
  finally { copying.value = false; }
  if (stopped) return;
  if (copied.size) {
    selection.value = failure ? new Set([...selection.value].filter((id) => !copied.has(id))) : new Set();
    await refreshMessages();
  }
  if (failure) {
    const message = failure instanceof Error ? failure.message : t('mail.error.copyFailed');
    if (identifiers.length === 1 && identifiers[0] === selectedId.value) detailError.value = message;
    else listError.value = message;
  }
}

/**
 * Apply server-backed primary flags in bounded batches, then reload the affected summaries.
 */
async function updateFlags(identifiers: string[], operation: MessageFlagOperation, flags: MessageFlags): Promise<void> {
  if (!identifiers.length) return;
  pendingFlagRequests.value += 1;
  let failure: unknown = null;
  try {
    for (let offset = 0; offset < identifiers.length; offset += MESSAGE_BATCH_SIZE) {
      await client.messages.setFlags({ identifiers: identifiers.slice(offset, offset + MESSAGE_BATCH_SIZE), operation, flags });
    }
  } catch (error) { failure = error; }
  finally { pendingFlagRequests.value -= 1; }
  if (stopped) return;
  await refreshMessages();
  if (failure) {
    const message = failure instanceof Error ? failure.message : t('mail.error.flagsFailed');
    if (selectedId.value && identifiers.includes(selectedId.value)) detailError.value = message;
    else listError.value = message;
  }
}

/**
 * Apply flags from the open message inspector.
 */
function updateDetailFlags(operation: MessageFlagOperation, flags: MessageFlags): void {
  if (detail.value) void updateFlags([detail.value.id], operation, flags)
}

/**
 * Moves messages to Trash at once, without confirmation, in API-sized batches. Batches trashed before a failure
 * leave the list and the selection; the failed rest stays selected and the failure is reported in the list.
 */
async function trashMessages(identifiers: string[]): Promise<void> {
  if (!identifiers.length || trashing.value) return;
  trashing.value = true;
  const trashed = new Set<string>();
  let failure: unknown = null;
  try {
    for (let offset = 0; offset < identifiers.length; offset += MESSAGE_BATCH_SIZE) {
      const batch = identifiers.slice(offset, offset + MESSAGE_BATCH_SIZE);
      await client.messages.trash(batch);
      for (const id of batch) trashed.add(id);
    }
  } catch (error) {
    failure = error;
  } finally { trashing.value = false; }
  if (stopped) return;
  if (trashed.size) {
    messages.value = messages.value.filter((message) => !trashed.has(message.id));
    selection.value = failure ? new Set([...selection.value].filter((id) => !trashed.has(id))) : new Set();
    if (selectedId.value && trashed.has(selectedId.value)) closeMessage();
    workspace.refreshInboxUnread();
    await refreshMessages();
  }
  if (failure) listError.value = failure instanceof Error ? failure.message : t('mail.error.trashFailed');
}

/**
 * Routes a Delete request: permanent deletion after confirmation inside Trash, an immediate move to Trash elsewhere.
 */
function requestDelete(ids: string[]): void {
  if (inTrash.value) confirmDelete(ids);
  else void trashMessages(ids);
}

/**
 * Opens the Empty Trash confirmation, counting the Trash only when every page is loaded.
 */
function confirmEmptyTrash(): void {
  if (emptyingTrash.value) return;
  emptyTrashCount.value = hasOlder.value ? null : messages.value.length;
  emptyTrashError.value = '';
  emptyTrashOpen.value = true;
}

/**
 * Permanently deletes everything in Trash, then reloads the emptied list; failures keep the dialog open.
 */
async function emptyTrash(): Promise<void> {
  if (emptyingTrash.value) return;
  emptyingTrash.value = true;
  emptyTrashError.value = '';
  try {
    await client.trash.empty();
    if (stopped) return;
    emptyTrashOpen.value = false;
    if (inTrash.value) {
      if (selectedId.value) closeMessage();
      selection.value = new Set();
      messages.value = [];
      hasOlder.value = false;
    }
    workspace.refreshInboxUnread();
    await refreshMessages();
  } catch (failure) {
    emptyTrashError.value = failure instanceof Error ? failure.message : t('mail.error.emptyTrashFailed');
  } finally { emptyingTrash.value = false; }
}

/**
 * Appends the next keyset page unless the folder or search changed during the request.
 */
async function loadOlder(): Promise<void> {
  if (loadingOlder.value || !hasOlder.value || !messages.value.length) return;
  loadingOlder.value = true;
  listError.value = null;
  const generation = listGeneration;
  try {
    const older = await fetchMessages(messages.value.reduce((oldest, message) => Math.min(oldest, message.sequence), Infinity));
    if (generation !== listGeneration || stopped) return;
    const existing = new Set(messages.value.map((message) => message.id));
    messages.value = [...messages.value, ...older.filter((message) => !existing.has(message.id))];
    hasOlder.value = older.length === PAGE_SIZE;
  } catch (error) {
    if (generation === listGeneration) listError.value = error instanceof Error ? error.message : t('mail.error.olderFailed');
  } finally { loadingOlder.value = false; }
}

/**
 * Loads the selected message and marks it read through the shared account status.
 */
async function loadDetail(id: string | null): Promise<void> {
  const request = ++detailRequest;
  detail.value = null;
  detailError.value = null;
  failedDownload.value = null;
  detailLoading.value = Boolean(id);
  detailSeen.value = messages.value.find((message) => message.id === id)?.seen ?? false;
  if (!id) return;
  try {
    const loaded = await client.messages.get(id);
    if (request !== detailRequest || stopped) return;
    detail.value = loaded;
    if (!selectedMessage.value?.seen) void updateSeen([id], true);
  } catch (error) {
    if (request === detailRequest) detailError.value = error instanceof Error ? error.message : t('mail.error.openFailed');
  } finally { if (request === detailRequest) detailLoading.value = false; }
}

/**
 * Change shared read status and update the visible list after the server accepts it.
 */
async function updateSeen(identifiers: string[], seen: boolean): Promise<void> {
  if (!identifiers.length) return;
  pendingSeenRequests.value += 1;
  try {
    for (let offset = 0; offset < identifiers.length; offset += MESSAGE_BATCH_SIZE) {
      const batch = identifiers.slice(offset, offset + MESSAGE_BATCH_SIZE);
      await workspace.setSeen(batch, seen);
      if (stopped) return;
      const changed = new Set(batch);
      if (selectedId.value && changed.has(selectedId.value)) detailSeen.value = seen;
      messages.value = messages.value.map((message) => changed.has(message.id) ? { ...message, seen, flags: message.flags ? { ...message.flags, seen } : undefined } : message);
    }
  } catch (failure) {
    if (stopped) return;
    const message = failure instanceof Error ? failure.message : t('mail.error.readFailed');
    if (selectedId.value && identifiers.includes(selectedId.value)) detailError.value = message;
    else listError.value = message;
  } finally { pendingSeenRequests.value -= 1; }
}

/**
 * Resets paginated list state when changing folder or search.
 */
function resetList(): void {
  listGeneration += 1;
  messages.value = [];
  hasOlder.value = false;
  void refreshMessages();
}

/**
 * The folder and search context carried by every message route.
 */
function listQuery() {
  const { folder: currentFolder, q } = route.query;
  return { ...(typeof currentFolder === 'string' ? { folder: currentFolder } : {}), ...(typeof q === 'string' ? { q } : {}) };
}

/**
 * Opens a selected email using a restorable route that keeps the folder and search.
 */
function selectMessage(id: string): void {
  if (id !== selectedId.value) void router.push({ name: 'mail-message', params: { id }, query: listQuery() });
  else if (!detail.value && !detailLoading.value) void loadDetail(id);
}

/**
 * Returns to the current list on narrow screens or after deletion.
 */
function closeMessage(): void {
  if (selectedId.value) void router.push({ name: 'mail', query: listQuery() });
}

/**
 * Clears an applied search and restores the folder it was started from.
 */
function clearAppliedSearch(): void {
  const { q: _removed, ...query } = route.query;
  void router.replace({ name: selectedId.value ? 'mail-message' : 'mail', params: route.params, query });
}

/**
 * Applies a selection from the list. A selection narrowed to one message opens it instead, so a lone highlighted
 * row always matches the reader.
 */
function updateSelection(next: Set<string>): void {
  const [only] = next;
  selection.value = next.size === 1 ? new Set() : next;
  if (next.size === 1 && only) selectMessage(only);
}

/**
 * Starts the explicit confirmation for permanently deleting one or more emails from Trash.
 */
function confirmDelete(ids: string[]): void {
  if (!ids.length || deleting.value) return;
  deleteIds.value = ids;
  deleteError.value = '';
  deleteOpen.value = true;
}

/**
 * Permanently deletes the confirmed emails and updates the list. When some deletions fail, the dialog stays open
 * on just those emails so Delete retries them, and they remain selected.
 */
async function deleteMessages(): Promise<void> {
  const ids = deleteIds.value;
  if (!ids.length || deleting.value) return;
  deleting.value = true;
  deleteError.value = '';
  deletedCount.value = 0;
  try {
    const { deleted, failed, error } = await deleteEach(client, ids, () => { deletedCount.value += 1; });
    if (stopped) return;
    if (selectedId.value && deleted.has(selectedId.value)) closeMessage();
    messages.value = messages.value.filter((message) => !deleted.has(message.id));
    selection.value = new Set([...selection.value].filter((id) => !deleted.has(id)));
    if (failed.length) {
      const reason = error instanceof Error ? error.message : t('mail.error.deleteFailed');
      deleteIds.value = failed;
      deleteError.value = ids.length === 1 ? reason : t('mail.error.deleteManyFailed', { failed: failed.length, count: ids.length, reason });
    } else deleteOpen.value = false;
    if (deleted.size) {
      workspace.refreshInboxUnread();
      await refreshMessages();
    }
  } finally { deleting.value = false; }
}

/**
 * Downloads bytes with a sanitized filename and releases the temporary browser URL.
 */
function saveDownload(bytes: Uint8Array, filename: string, contentType: string): void {
  const url = URL.createObjectURL(new Blob([new Uint8Array(bytes).buffer], { type: contentType }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename.replace(/[\\/\x00-\x1f]/g, '_');
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/**
 * Downloads the original message without changing captured mail.
 */
async function downloadRaw(): Promise<void> {
  const id = selectedId.value;
  if (!id) return;
  detailError.value = null;
  failedDownload.value = null;
  try { saveDownload(await client.messages.getRaw(id), `sandpost-${id}.eml`, 'message/rfc822'); }
  catch (error) {
    if (id === selectedId.value) {
      failedDownload.value = 'raw';
      detailError.value = error instanceof Error ? error.message : t('mail.error.downloadOriginalFailed');
    }
  }
}

/**
 * Downloads one decoded MIME attachment rather than the entire original message.
 */
async function downloadAttachment(index: number): Promise<void> {
  const message = detail.value;
  const attachment = message?.attachments[index];
  if (!message || !attachment || downloadingAttachment.value !== null) return;
  downloadingAttachment.value = index;
  detailError.value = null;
  failedDownload.value = null;
  try { saveDownload(await client.messages.getAttachment(message.id, index), attachment.filename || `attachment-${index + 1}`, attachment.contentType); }
  catch (error) {
    if (message.id === selectedId.value) {
      failedDownload.value = index;
      detailError.value = error instanceof Error ? error.message : t('mail.error.downloadAttachmentFailed');
    }
  }
  finally { downloadingAttachment.value = null; }
}

/**
 * Retries the operation that failed while preserving the currently selected email.
 */
function retryInspection(): void {
  if (detail.value && failedDownload.value === 'raw') void downloadRaw();
  else if (detail.value && typeof failedDownload.value === 'number') void downloadAttachment(failedDownload.value);
  else void loadDetail(selectedId.value);
}

watch(listKey, () => {
  if (!onMailRoute.value) return;
  unreadOnly.value = false;
  selection.value = new Set();
  resetList();
});
/*
 * A folder that disappeared (deleted elsewhere, or no longer visible) falls back to the inbox. Once a pending
 * folder resolves, its kind changes the list key, which loads it.
 */
watch([folderPending, folder], ([pending, current]) => {
  if (onMailRoute.value && !pending && !current) void router.replace({ name: 'mail', query: appliedSearch.value ? { q: appliedSearch.value } : {} });
}, { immediate: true });
/*
 * Messages that leave the list (deleted elsewhere, or hidden by the Unread filter) leave the selection too.
 */
watch(visibleMessages, (visible) => {
  if (!selection.value.size) return;
  const listed = new Set(visible.map((message) => message.id));
  if ([...selection.value].some((id) => !listed.has(id))) selection.value = new Set([...selection.value].filter((id) => listed.has(id)));
});
watch(selectedId, (id) => { if (onMailRoute.value) void loadDetail(id); }, { immediate: true });
onMounted(() => {
  void refreshMessages();
  unsubscribeEvents = workspace.onMailEvent((event) => {
    if (event.type !== 'disconnected') void refreshMessages();
  });
});
onBeforeUnmount(() => {
  stopped = true;
  detailRequest += 1;
  unsubscribeEvents?.();
});
</script>

<template>
  <div class="grid h-full min-h-0 grid-cols-1 overflow-hidden lg:grid-cols-[minmax(19rem,22rem)_minmax(0,1fr)] xl:grid-cols-[minmax(22rem,26rem)_minmax(0,1fr)]">
    <MessageList
      :class="cn('min-h-0 lg:border-r', selectedId && !multipleSelected && 'max-lg:hidden')"
      :folder="folder"
      :folders="workspace.folders.value"
      :search="appliedSearch"
      :messages="visibleMessages"
      :selected-id="selectedId"
      :selection="selection"
      :unread-only="unreadOnly"
      :read-message-ids="readMessageIds"
      :seen-busy="pendingSeenRequests > 0"
      :loading="loading || folderPending"
      :refreshing="refreshing"
      :loading-older="loadingOlder"
      :has-older="hasOlder"
      :error="listError"
      :show-empty-trash="canDeleteMail"
      :empty-trash-disabled="!messages.length || loading || refreshing || trashing || deleting || emptyingTrash"
      @clear-search="clearAppliedSearch"
      @filter="unreadOnly = $event"
      @select="selectMessage"
      @refresh="refreshMessages"
      @older="loadOlder"
      @update:selection="updateSelection"
      @delete="requestDelete"
      @move="moveMessages"
      @set-seen="updateSeen"
      @empty-trash="confirmEmptyTrash"
    />
    <MessageSelectionPanel
      v-if="multipleSelected"
      class="min-h-0 max-lg:hidden"
      :count="selectedInOrder.length"
      :deleting="deleting || trashing || copying || pendingFlagRequests > 0"
      :folders="workspace.folders.value"
      :flags="selectedFlags"
      :trash="inTrash"
      @delete="requestDelete(selectedInOrder)"
      @move="moveMessages(selectedInOrder, $event)"
      @copy="copyMessages(selectedInOrder, $event)"
      @set-flags="(operation, flags) => updateFlags(selectedInOrder, operation, flags)"
      @clear="selection = new Set()"
    />
    <MessageDetailPane
      v-else
      :class="cn('min-h-0', !selectedId && 'max-lg:hidden')"
      :message="detail"
      :seen="selectedMessage?.seen ?? detailSeen"
      :flags="selectedMessage?.flags"
      :seen-busy="pendingSeenRequests > 0"
      :loading="detailLoading"
      :error="detailError"
      :mobile="true"
      :downloading-attachment="downloadingAttachment"
      :deleting="deleting || trashing || copying || pendingFlagRequests > 0"
      :folders="workspace.folders.value"
      :trash="inTrash"
      @back="closeMessage"
      @retry="retryInspection"
      @download="downloadRaw"
      @attachment="downloadAttachment"
      @delete="detail && requestDelete([detail.id])"
      @move="detail && moveMessages([detail.id], $event)"
      @copy="detail && copyMessages([detail.id], $event)"
      @set-flags="updateDetailFlags"
      @set-seen="detail && updateSeen([detail.id], $event)"
    />

    <BusyDialog v-model:open="deleteOpen" :title="deleteTitle" :description="deleteDescription" :busy="deleting">
      <ul class="flex flex-col gap-1 rounded-lg border bg-muted/40 px-3 py-2">
        <li v-for="(subject, index) in deleteSubjects" :key="index" :class="deleteIds.length === 1 ? 'break-words' : 'truncate'">{{ subject }}</li>
        <li v-if="deleteIds.length > DELETE_SUBJECT_PREVIEW" class="text-muted-foreground">{{ t('mail.delete.andMore', { count: deleteIds.length - DELETE_SUBJECT_PREVIEW }) }}</li>
      </ul>
      <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
      <DialogFooter>
        <Button variant="outline" :disabled="deleting" @click="deleteOpen = false">{{ t('mail.common.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="deleteMessages">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />
          {{ deleting ? (deleteIds.length > 1 ? t('mail.delete.deletingMany', { deleted: deletedCount, count: deleteIds.length }) : t('mail.delete.deletingOne')) : deleteIds.length > 1 ? t('mail.delete.deleteMany', { count: deleteIds.length }) : t('mail.list.deletePermanently') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
    <BusyDialog v-model:open="emptyTrashOpen" :title="t('mail.delete.emptyTitle')" :description="emptyTrashDescription" :busy="emptyingTrash">
      <Alert v-if="emptyTrashError" variant="destructive" role="alert"><AlertDescription>{{ emptyTrashError }}</AlertDescription></Alert>
      <DialogFooter>
        <Button variant="outline" :disabled="emptyingTrash" @click="emptyTrashOpen = false">{{ t('mail.common.cancel') }}</Button>
        <Button variant="destructive" :disabled="emptyingTrash" @click="emptyTrash">
          <Spinner v-if="emptyingTrash" data-icon="inline-start" aria-hidden="true" />{{ emptyingTrash ? t('mail.delete.emptying') : t('mail.delete.empty') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
  </div>
</template>
