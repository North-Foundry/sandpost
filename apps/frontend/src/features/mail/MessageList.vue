<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, nextTick, onBeforeUnmount, reactive, ref, useTemplateRef, watch } from 'vue'
import { useEventListener } from '@vueuse/core'
import {
  CheckIcon, CopyIcon, LinkIcon, PaperclipIcon, RefreshCwIcon, SearchIcon, SearchXIcon,
  MailIcon, MailOpenIcon, Trash2Icon, XIcon, ActivityIcon, InfoIcon, PencilIcon,
} from '@lucide/vue'
import type { MessageSummary } from '@/api/contracts'
import { formatFullDate, formatListDate, sendersLabel } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertAction, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import QueryHighlight from '@/features/query/QueryHighlight.vue'
import { looksLikeCanonicalQuery } from '@/features/query/vocabulary'
import type { MailFolder } from '@/features/shell/workspace'
import { folderIcon } from './folderIcons'
import { extendSelection, orderedSelection, settleSwipe, toggleSelection, type MessageSelection } from './messageSelection'
import MoveToMenu from './MoveToMenu.vue'
import { useCopy } from '@/composables/useCopy'

/**
 * Start fetching the next page while this many pixels of rows remain below the viewport.
 */
const PREFETCH_DISTANCE = 600

/**
 * Width in pixels of the Delete action a partial swipe leaves revealed.
 */
const SWIPE_REVEAL_WIDTH = 84

/**
 * A touch or pen drag must travel this far before it is classified as a horizontal swipe or a vertical scroll.
 */
const SWIPE_DRAG_SLOP = 8

/**
 * Trackpad wheel gestures end, and settle, after this many milliseconds without a wheel event.
 */
const WHEEL_GESTURE_IDLE = 140

/**
 * Matches the row's settle transition, after which a closed swipe stops rendering its action.
 */
const SWIPE_SETTLE_DURATION = 200

const props = defineProps<{
  messages: MessageSummary[]
  selectedId: string | null

  /**
   * Messages chosen for a bulk action; empty while only the open message is active.
   */
  selection: MessageSelection

  /**
   * The open folder; null while it is still being resolved.
   */
  folder: MailFolder | null
  folders: MailFolder[]

  /**
   * The applied search as typed, or empty for the folder's own list.
   */
  search: string
  unreadOnly: boolean
  readMessageIds: Set<string>
  seenBusy?: boolean
  loading: boolean
  refreshing: boolean
  loadingOlder: boolean
  hasOlder: boolean
  error: string | null

  /**
   * Offer Empty Trash while Trash is open; viewers cannot delete mail and are not offered it.
   */
  showEmptyTrash?: boolean
  emptyTrashDisabled?: boolean
  canMutate?: boolean

  checkingLinks?: boolean

  /**
   * Confirmation of the last bulk link check request, announced to assistive technology.
   */
  linkCheckNotice?: string

  /**
   * The signed-in user, who may edit Personas they own.
   */
  currentUserId?: string | null
}>()
const { t, locale } = useI18n()
const emit = defineEmits<{
  /**
   * Requests changing the messages’ seen state.
   */
  (event: 'set-seen', identifiers: string[], seen: boolean): void

  /**
   * Emits the selected message identifier.
   */
  (event: 'select', id: string): void

  /**
   * Requests clearing the applied search.
   */
  (event: 'clearSearch'): void

  /**
   * Requests refreshing the message list.
   */
  (event: 'refresh'): void

  /**
   * Requests loading the next older page.
   */
  (event: 'older'): void

  /**
   * Requests changing whether only unread messages are shown.
   */
  (event: 'filter', value: boolean): void

  /**
   * Emits the updated bulk selection.
   */
  (event: 'update:selection', value: Set<string>): void

  /**
   * Requests deletion of these messages, in list order: the page moves them to Trash at once, or inside Trash
   * confirms before deleting them permanently.
   */
  (event: 'delete', ids: string[]): void

  /**
   * Requests moving the applicable messages to the destination.
   */
  (event: 'move', ids: string[], folderIdentifier: string | null): void

  /**
   * Requests permanently emptying Trash.
   */
  (event: 'emptyTrash'): void

  /**
   * Requests checking the links of the selected messages.
   */
  (event: 'check-links', ids: string[]): void

  /**
   * Requests the open Persona's run history.
   */
  (event: 'persona-activity'): void

  /**
   * Requests editing the open Persona.
   */
  (event: 'persona-edit'): void
}>()
const scroller = useTemplateRef<HTMLElement>('scroller')
const { copied, error: copyError, copy, reset: resetCopy } = useCopy<'address'>(t('mail.address.addressCopyError'))
const filter = computed(() => props.unreadOnly ? 'unread' : 'all')

/**
 * Only canonical searches are highlighted; shortcut and plain text are echoed as typed.
 */
const canonicalSearch = computed(() => looksLikeCanonicalQuery(props.search))

/**
 * Search results never include trashed mail, so only the Trash's own list deletes permanently.
 */
const inTrash = computed(() => !props.search && props.folder?.kind === 'trash')
const title = computed(() => props.search ? t('mail.list.searchResults') : props.folder?.name ?? t('mail.list.inbox'))
const icon = computed(() => props.search ? SearchIcon : folderIcon(props.folder?.view?.icon ?? props.folder?.manual?.icon, props.folder?.kind ?? 'inbox'))

/**
 * Only a Persona's owner edits it; administrators may still read its activity.
 */
const personaEditable = computed(() => Boolean(props.canMutate && props.folder?.persona && props.folder.persona.ownerId === props.currentUserId))
const countLabel = computed(() => {
  const count = props.messages.length
  const plus = props.hasOlder ? '+' : ''
  return t(count === 1 && !props.hasOlder ? 'mail.list.messageCount_one' : 'mail.list.messageCount_other', { count, plus })
})

/**
 * Display strings are derived once per list change rather than on every row render.
 */
const rows = computed(() => {
  void locale.value
  const now = new Date()
  return props.messages.map((message) => ({
    id: message.id,
    sender: sendersLabel(message.from),
    senderTitle: message.from.join(', '),
    date: formatListDate(message.receivedAt, now),
    dateTitle: formatFullDate(message.receivedAt),
    subject: message.subject || '(no subject)',
    preview: message.preview || `To: ${message.to.join(', ') || 'No recipients'}`,
    attachments: message.attachmentCount,
    unread: !props.readMessageIds.has(message.id),
  }))
})

const rowOrder = computed(() => rows.value.map((row) => row.id))
const multipleSelected = computed(() => props.selection.size > 1)
const selectionSeen = computed(() => [...props.selection].every((id) => props.readMessageIds.has(id)))

/**
 * The row Shift-selection extends from: the last row clicked without Shift, else the open message.
 */
const selectionAnchor = ref<string | null>(null)

/**
 * The single row currently swiped open, or being swiped. Only one row is swiped at a time.
 */
const swipe = reactive({ id: null as string | null, offset: 0, rowWidth: 0, dragging: false })

/**
 * An in-progress touch or pen drag; `axis` stays undecided until the pointer passes the drag slop.
 */
let pointerDrag: { pointerId: number; startX: number; startY: number; startOffset: number; axis: 'undecided' | 'horizontal' | 'vertical' } | null = null

/**
 * The axis a trackpad gesture locked onto, so a vertical scroll never drifts into a swipe.
 */
let wheelAxis: 'horizontal' | 'vertical' | null = null
let wheelIdleTimer: ReturnType<typeof setTimeout> | undefined
let swipeCloseTimer: ReturnType<typeof setTimeout> | undefined

/**
 * A drag ends with a click on the row it started on; that click must not open the message.
 */
let suppressNextClick = false

/**
 * After a Delete keypress, the row that takes keyboard focus once the deleted rows leave the list, so repeated
 * presses keep working through the mailbox as in macOS Mail.
 */
let focusAfterDelete: { removed: Set<string>; next: string | null } | null = null

/**
 * Highlights rows in the bulk selection, or the open message when nothing is bulk-selected.
 */
function isHighlighted(id: string): boolean {
  return props.selection.size ? props.selection.has(id) : id === props.selectedId
}

/**
 * The messages a Delete keypress acts on: the bulk selection, else the open or focused message.
 */
function deletionTargets(): string[] {
  if (props.selection.size) return orderedSelection(rowOrder.value, props.selection)
  const focusedId = (document.activeElement as HTMLElement | null)?.dataset.messageId
  const id = props.selectedId ?? focusedId
  return id && rowOrder.value.includes(id) ? [id] : []
}

/**
 * The row after the last deleted one, or else the row before the first, which inherits keyboard focus.
 */
function rowAfterDeletion(ids: string[]): string | null {
  const removed = new Set(ids)
  const order = rowOrder.value
  const last = Math.max(...ids.map((id) => order.indexOf(id)))
  const first = Math.min(...ids.map((id) => order.indexOf(id)))
  return order.slice(last + 1).find((id) => !removed.has(id)) ?? order.slice(0, first).reverse().find((id) => !removed.has(id)) ?? null
}

/**
 * Opens a message with a plain click or Shift/Cmd-clicks it into the bulk selection, as in macOS Mail.
 */
function onRowClick(event: MouseEvent, id: string): void {
  if (suppressNextClick) { suppressNextClick = false; return }
  if (swipe.id) { closeSwipe(); return }
  if (event.shiftKey) {
    emit('update:selection', extendSelection(rowOrder.value, props.selection, selectionAnchor.value ?? props.selectedId, id, event.metaKey || event.ctrlKey))
    return
  }
  selectionAnchor.value = id
  if (event.metaKey || event.ctrlKey) {
    emit('update:selection', toggleSelection(props.selection, id, props.selectedId))
    return
  }
  if (props.selection.size) emit('update:selection', new Set())
  emit('select', id)
}

/**
 * Selects every loaded row.
 */
function selectAll(): void {
  emit('update:selection', new Set(rowOrder.value))
}

/**
 * Focuses a row by id once it has rendered.
 */
function focusRow(id: string): void {
  void nextTick(() => scroller.value?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(id)}"]`)?.focus())
}

/**
 * Applies the All/Unread filter while ignoring attempts to deselect the active item.
 */
function setFilter(value: unknown): void {
  if (value === 'all' || value === 'unread') emit('filter', value === 'unread')
}

/**
 * Copies the address inbox's receiving address, briefly confirming success or reporting failures.
 */
async function copyAddress(): Promise<void> {
  await copy(props.folder?.address || '', 'address')
}

/**
 * Request the next keyset page once the reader nears the end of the loaded rows.
 */
function loadMoreIfNear(): void {
  const element = scroller.value
  if (!element || !props.hasOlder || props.loadingOlder || props.error || !props.messages.length) return
  if (element.scrollHeight - element.scrollTop - element.clientHeight < PREFETCH_DISTANCE) emit('older')
}

/**
 * List keyboard commands: arrows or j/k move the open message, Shift+arrows extend the bulk selection,
 * Cmd/Ctrl+A selects every loaded row, Delete/Backspace deletes the selection (to Trash, or permanently inside
 * Trash), and Escape closes a swiped row or clears the selection.
 */
function onKeydown(event: KeyboardEvent): void {
  const command = event.metaKey || event.ctrlKey
  if (command && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'a') {
    event.preventDefault()
    selectAll()
    return
  }
  if (props.canMutate && (event.key === 'Delete' || event.key === 'Backspace') && !event.altKey && !event.shiftKey) {
    const ids = deletionTargets()
    if (!ids.length) return
    event.preventDefault()
    focusAfterDelete = { removed: new Set(ids), next: rowAfterDeletion(ids) }
    emit('delete', ids)
    return
  }
  if (event.key === 'Escape' && (swipe.id || props.selection.size)) {
    event.preventDefault()
    if (swipe.id) closeSwipe()
    else emit('update:selection', new Set())
    return
  }
  if (command || event.altKey) return
  const step = event.key === 'ArrowDown' || event.key === 'j' ? 1 : event.key === 'ArrowUp' || event.key === 'k' ? -1 : 0
  if (!step || !rows.value.length) return
  event.preventDefault()
  const focusedId = (document.activeElement as HTMLElement | null)?.dataset.messageId
  const current = rows.value.findIndex((row) => row.id === (focusedId ?? props.selectedId))
  const next = rows.value[Math.min(rows.value.length - 1, Math.max(0, current < 0 ? 0 : current + step))]
  if (!next) return
  if (event.shiftKey) {
    /*
     * The anchor stays put so repeated Shift+arrows grow or shrink one contiguous range.
     */
    selectionAnchor.value ??= props.selectedId ?? rows.value[current]?.id ?? next.id
    emit('update:selection', extendSelection(rowOrder.value, props.selection, selectionAnchor.value, next.id, false))
  } else {
    selectionAnchor.value = next.id
    if (props.selection.size) emit('update:selection', new Set())
    emit('select', next.id)
  }
  focusRow(next.id)
}

/**
 * Moves the swiped row to `offset` pixels left of rest, clamped to the row's width.
 */
function setSwipeOffset(offset: number): void {
  swipe.offset = Math.min(Math.max(offset, 0), swipe.rowWidth || Infinity)
}

/**
 * Starts swiping `id`, closing any other row that was left open.
 */
function beginSwipe(id: string, rowElement: HTMLElement): void {
  clearTimeout(swipeCloseTimer)
  if (swipe.id !== id) { swipe.id = id; swipe.offset = 0 }
  swipe.rowWidth = rowElement.clientWidth
  swipe.dragging = true
}

/**
 * Animates the swiped row back to rest and stops rendering its action once the transition ends.
 */
function closeSwipe(): void {
  if (!swipe.id) return
  swipe.offset = 0
  swipe.dragging = false
  clearTimeout(swipeCloseTimer)
  swipeCloseTimer = setTimeout(() => { if (swipe.offset === 0) swipe.id = null }, SWIPE_SETTLE_DURATION)
}

/**
 * Settles a released swipe: back to rest, open on the Delete action, or a full-swipe delete request.
 */
function releaseSwipe(): void {
  if (!props.canMutate) return
  const id = swipe.id
  if (!id) return
  swipe.dragging = false
  const outcome = settleSwipe(swipe.offset, swipe.rowWidth, SWIPE_REVEAL_WIDTH)
  if (outcome === 'reveal') { swipe.offset = SWIPE_REVEAL_WIDTH; return }
  closeSwipe()
  if (outcome === 'delete') emit('delete', [id])
}

/**
 * Requests deletion of the row whose revealed Delete action was pressed.
 */
function deleteSwipedRow(id: string): void {
  if (!props.canMutate) return
  closeSwipe()
  emit('delete', [id])
}

/**
 * Two-finger trackpad swipes arrive as horizontal wheel events. Swiping left (positive deltaX) slides the row
 * open; the gesture settles once the wheel, including macOS momentum, goes quiet.
 */
function onRowWheel(event: WheelEvent, id: string): void {
  if (!wheelAxis) wheelAxis = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? 'horizontal' : 'vertical'
  clearTimeout(wheelIdleTimer)
  wheelIdleTimer = setTimeout(() => {
    wheelAxis = null
    if (swipe.dragging && !pointerDrag) releaseSwipe()
  }, WHEEL_GESTURE_IDLE)
  if (wheelAxis !== 'horizontal') return
  event.preventDefault()
  if (swipe.id !== id || !swipe.dragging) beginSwipe(id, event.currentTarget as HTMLElement)
  const lineHeight = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1
  setSwipeOffset(swipe.offset + event.deltaX * lineHeight)
}

/**
 * Tracks a touch or pen press as a possible swipe; mouse presses keep their click and text behavior.
 */
function onRowPointerDown(event: PointerEvent, id: string): void {
  if (event.pointerType === 'mouse' || !event.isPrimary) return
  if (swipe.id && swipe.id !== id) closeSwipe()
  pointerDrag = {
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    startOffset: swipe.id === id ? swipe.offset : 0,
    axis: 'undecided',
  }
}

/**
 * Follows a horizontal drag with the row once the pointer clearly moves sideways.
 */
function onRowPointerMove(event: PointerEvent, id: string): void {
  if (!pointerDrag || event.pointerId !== pointerDrag.pointerId) return
  const horizontal = pointerDrag.startX - event.clientX
  const vertical = event.clientY - pointerDrag.startY
  if (pointerDrag.axis === 'undecided') {
    if (Math.max(Math.abs(horizontal), Math.abs(vertical)) < SWIPE_DRAG_SLOP) return
    pointerDrag.axis = Math.abs(horizontal) > Math.abs(vertical) ? 'horizontal' : 'vertical'
    if (pointerDrag.axis === 'vertical') return
    const rowElement = event.currentTarget as HTMLElement
    rowElement.setPointerCapture(event.pointerId)
    beginSwipe(id, rowElement)
  }
  if (pointerDrag.axis === 'horizontal') setSwipeOffset(pointerDrag.startOffset + horizontal)
}

/**
 * Ends a touch or pen drag, settling the row and swallowing the click the release produces.
 */
function onRowPointerUp(event: PointerEvent): void {
  if (!pointerDrag || event.pointerId !== pointerDrag.pointerId) return
  const swiped = pointerDrag.axis === 'horizontal'
  pointerDrag = null
  if (!swiped) return
  suppressNextClick = event.type === 'pointerup'
  releaseSwipe()
}

/**
 * Closes an open swipe when the reader presses anywhere outside that row.
 */
function closeSwipeOnOutsidePress(event: PointerEvent): void {
  if (!swipe.id || swipe.dragging) return
  const row = (event.target as Element | null)?.closest('[data-swipe-row]')
  if ((row as HTMLElement | null)?.dataset.swipeRow !== swipe.id) closeSwipe()
}

useEventListener(scroller, 'scroll', () => {
  loadMoreIfNear()
  if (swipe.id && !swipe.dragging) closeSwipe()
}, { passive: true })
useEventListener(document, 'pointerdown', closeSwipeOnOutsidePress, { capture: true })
/*
 * A list change can remove the swiped row, so any open swipe is dropped rather than left pointing at it.
 */
watch(rowOrder, (order) => { if (swipe.id && !order.includes(swipe.id)) { swipe.id = null; swipe.offset = 0 } })
/*
 * Once every deleted row has left, focus moves to its neighbor; a cancelled confirmation leaves the rows and focus be.
 */
watch(rowOrder, (order) => {
  if (!focusAfterDelete || order.some((id) => focusAfterDelete!.removed.has(id))) return
  const next = focusAfterDelete.next
  focusAfterDelete = null
  if (next && order.includes(next)) focusRow(next)
})
watch(() => props.folder?.id, () => { selectionAnchor.value = null; focusAfterDelete = null })
onBeforeUnmount(() => { clearTimeout(wheelIdleTimer); clearTimeout(swipeCloseTimer) })
/*
 * Short pages and the Unread filter can leave the viewport unfilled, so check again after each change.
 */
watch(() => [props.messages.length, props.loadingOlder, props.hasOlder, props.error], () => { void nextTick(loadMoreIfNear) })
watch(() => props.folder?.id, resetCopy)
</script>

<template>
  <section class="flex min-h-0 min-w-0 flex-col bg-background" :aria-label="t('mail.list.messages')">
    <header class="flex shrink-0 flex-col gap-1 border-b px-3 py-2">
      <div class="flex min-w-0 items-center gap-2">
        <component :is="icon" class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <h1 class="min-w-0 truncate text-sm font-semibold">{{ title }}</h1>
        <span class="shrink-0 text-xs text-muted-foreground tabular-nums">{{ props.loading ? '' : countLabel }}</span>
        <div class="ml-auto flex shrink-0 items-center gap-1">
          <ToggleGroup type="single" variant="outline" size="sm" class="h-7" :model-value="filter" :aria-label="t('mail.list.filter')" @update:model-value="setFilter">
            <ToggleGroupItem value="all" class="h-7 px-2 text-xs">{{ t('mail.list.all') }}</ToggleGroupItem>
            <ToggleGroupItem value="unread" class="h-7 px-2 text-xs" :title="t('mail.list.unreadTitle')">{{ t('mail.list.unread') }}</ToggleGroupItem>
          </ToggleGroup>
          <Button variant="ghost" size="icon-sm" :aria-label="t('mail.list.refresh')" :title="t('mail.list.refresh')" :disabled="props.refreshing" @click="emit('refresh')">
            <RefreshCwIcon :class="cn(props.refreshing && 'animate-spin')" />
          </Button>
        </div>
      </div>
      <!--
        Context line: what this list contains.
      -->
      <div v-if="props.selection.size" class="flex min-h-5 min-w-0 items-center gap-1 text-xs" role="toolbar" :aria-label="t('mail.list.selected')">
        <span class="font-medium text-foreground tabular-nums" role="status">{{ t('mail.list.selectedCount', { count: props.selection.size }) }}</span>
        <Button v-if="props.selection.size < rows.length" variant="ghost" size="xs" class="text-muted-foreground" @click="selectAll">{{ t('mail.list.selectAll') }}</Button>
        <MoveToMenu v-if="props.canMutate" :folders="props.folders" @move="emit('move', orderedSelection(rowOrder, props.selection), $event)" />
        <Button v-if="props.canMutate" variant="ghost" size="icon-xs" :disabled="props.seenBusy" :aria-label="selectionSeen ? t('mail.list.markUnread') : t('mail.list.markRead')" :title="selectionSeen ? t('mail.list.markUnread') : t('mail.list.markRead')" @click="emit('set-seen', orderedSelection(rowOrder, props.selection), !selectionSeen)">
          <MailOpenIcon v-if="selectionSeen" /><MailIcon v-else />
        </Button>
        <Button v-if="props.canMutate"
          variant="ghost"
          size="xs"
          class="text-destructive hover:bg-destructive/10 hover:text-destructive"
          :aria-label="inTrash ? t('mail.list.deleteSelected') : t('mail.list.trashSelected')"
          :title="inTrash ? t('mail.list.deletePermanently') : t('mail.list.moveToTrash')"
          @click="emit('delete', orderedSelection(rowOrder, props.selection))"
        >
          <Trash2Icon data-icon="inline-start" />{{ inTrash ? t('mail.list.delete') : t('mail.list.trash') }}
        </Button>
        <Button variant="ghost" size="icon-xs" :disabled="props.checkingLinks" :aria-label="t('mail.links.bulk')" :title="props.linkCheckNotice || t('mail.links.bulk')" @click="emit('check-links', orderedSelection(rowOrder, props.selection))">
          <Spinner v-if="props.checkingLinks" aria-hidden="true" /><CheckIcon v-else-if="props.linkCheckNotice" class="text-success" /><LinkIcon v-else />
        </Button>
        <span class="sr-only" role="status">{{ props.linkCheckNotice }}</span>
        <Button variant="ghost" size="icon-xs" :aria-label="t('mail.list.clearSelection')" :title="t('mail.list.clearSelectionEsc')" @click="emit('update:selection', new Set())"><XIcon /></Button>
      </div>
      <div v-else class="flex min-h-5 min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
        <template v-if="props.search">
          <code class="min-w-0 truncate rounded bg-muted px-1.5 py-px font-mono text-foreground" :title="props.search"><QueryHighlight v-if="canonicalSearch" :source="props.search" /><template v-else>{{ props.search }}</template></code>
          <span class="shrink-0">{{ t('mail.list.allMail') }}</span>
          <Button variant="ghost" size="icon-xs" class="ml-auto" :aria-label="t('mail.list.clearSearch')" :title="t('mail.list.clearSearch')" @click="emit('clearSearch')"><XIcon /></Button>
        </template>
        <template v-else-if="props.folder?.kind === 'address'">
          <code class="min-w-0 truncate font-mono select-all" :title="props.folder.address ?? undefined">{{ props.folder.address }}</code>
          <Button variant="ghost" size="icon-xs" :aria-label="copied ? t('mail.list.addressCopied') : t('mail.list.copyAddress')" :title="copied ? t('mail.list.copied') : t('mail.list.copyAddress')" @click="copyAddress">
            <CheckIcon v-if="copied" class="text-success" /><CopyIcon v-else />
          </Button>
        </template>
        <template v-else-if="props.folder?.kind === 'persona' && props.folder.persona">
          <code class="min-w-0 truncate font-mono select-all" :title="props.folder.address ?? undefined">{{ props.folder.address }}</code>
          <Button variant="ghost" size="icon-xs" :aria-label="copied ? t('mail.list.addressCopied') : t('mail.list.copyAddress')" :title="copied ? t('mail.list.copied') : t('mail.list.copyAddress')" @click="copyAddress">
            <CheckIcon v-if="copied" class="text-success" /><CopyIcon v-else />
          </Button>
          <span v-if="!props.folder.persona.enabled" class="shrink-0 rounded-sm border px-1.5 py-px text-[11px] leading-4">{{ t('personas.list.disabled') }}</span>
          <span
            v-if="props.folder.persona.addressAccess !== 'confirmed'"
            class="flex shrink-0 items-center"
            :title="props.folder.persona.addressAccess === 'denied' ? t('personas.access.denied') : t('personas.access.perMessage')"
          >
            <InfoIcon class="size-3.5" aria-hidden="true" />
            <span class="sr-only">{{ props.folder.persona.addressAccess === 'denied' ? t('personas.access.denied') : t('personas.access.perMessage') }}</span>
          </span>
          <span class="ml-auto flex shrink-0 items-center gap-0.5">
            <Button variant="ghost" size="xs" @click="emit('persona-activity')"><ActivityIcon data-icon="inline-start" />{{ t('personas.list.activity') }}</Button>
            <Button v-if="personaEditable" variant="ghost" size="xs" @click="emit('persona-edit')"><PencilIcon data-icon="inline-start" />{{ t('personas.list.edit') }}</Button>
          </span>
        </template>
        <code v-else-if="props.folder?.kind === 'smart'" class="min-w-0 truncate font-mono" :title="props.folder.view?.filter">
          <QueryHighlight v-if="props.folder.view?.filter.trim()" :source="props.folder.view.filter" /><span v-else>{{ t('mail.list.allReadable') }}</span>
        </code>
        <template v-else-if="inTrash">
          <span class="min-w-0 truncate">{{ t('mail.list.deletedMessages') }}</span>
          <Button
            v-if="props.showEmptyTrash"
            variant="ghost"
            size="xs"
            class="ml-auto shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
            :disabled="props.emptyTrashDisabled"
            @click="emit('emptyTrash')"
          >
            {{ t('mail.list.emptyTrash') }}
          </Button>
        </template>
        <span v-else class="truncate">{{ t('mail.list.allMailReadable') }}</span>
      </div>
      <p v-if="copyError" class="text-xs text-destructive" role="status">{{ copyError }}</p>
    </header>

    <Empty v-if="props.error && !props.messages.length" role="alert">
      <EmptyHeader>
        <EmptyMedia variant="icon" class="bg-destructive/10 text-destructive"><RefreshCwIcon /></EmptyMedia>
        <EmptyTitle>{{ props.search ? t('mail.list.searchFailed') : t('mail.list.loadFailed') }}</EmptyTitle>
        <EmptyDescription class="break-words">{{ props.error }}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button variant="outline" size="sm" @click="emit('refresh')">{{ t('mail.list.tryAgain') }}</Button>
        <Button v-if="props.search" variant="ghost" size="sm" @click="emit('clearSearch')">{{ t('mail.list.clearSearch') }}</Button>
      </EmptyContent>
    </Empty>

    <div v-else-if="props.loading && !props.messages.length" class="flex flex-col" role="status" :aria-label="t('mail.list.loading')">
      <div v-for="index in 8" :key="index" class="flex flex-col gap-2 border-b border-border/60 px-4 py-3">
        <div class="flex justify-between gap-6"><Skeleton class="h-3.5 w-1/3" /><Skeleton class="h-3 w-10" /></div>
        <Skeleton class="h-3 w-4/5" />
      </div>
    </div>

    <Empty v-else-if="!props.messages.length">
      <EmptyHeader>
        <EmptyMedia variant="icon"><SearchXIcon v-if="props.search" /><component :is="icon" v-else /></EmptyMedia>
        <EmptyTitle>{{ props.search ? t('mail.list.noMatches') : props.unreadOnly ? t('mail.list.noUnread') : inTrash ? t('mail.list.trashEmpty') : t('mail.list.noMessages') }}</EmptyTitle>
        <EmptyDescription>
          {{ props.search
            ? t('mail.list.trySearch')
            : props.unreadOnly
              ? t('mail.list.allViewed')
              : inTrash
                ? t('mail.list.deletedRemain')
                : props.folder?.kind === 'smart'
                  ? t('mail.list.smartFolderEmpty')
                  : t('mail.list.messagesArrive') }}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button v-if="props.folder?.address && !props.search && !props.unreadOnly" variant="outline" size="sm" @click="copyAddress">
          <CheckIcon v-if="copied" data-icon="inline-start" class="text-success" /><CopyIcon v-else data-icon="inline-start" />{{ copied ? t('mail.list.addressCopied') : t('mail.list.copyAddress') }}
        </Button>
        <Button v-if="props.search" variant="outline" size="sm" @click="emit('clearSearch')">{{ t('mail.list.clearSearch') }}</Button>
        <Button v-if="props.unreadOnly" variant="outline" size="sm" @click="emit('filter', false)">{{ t('mail.list.showAll') }}</Button>
        <Button v-if="props.hasOlder" variant="ghost" size="sm" :disabled="props.loadingOlder" @click="emit('older')">{{ props.loadingOlder ? t('mail.list.loadingShort') : t('mail.list.loadOlder') }}</Button>
      </EmptyContent>
    </Empty>

    <div
      v-else
      ref="scroller"
      class="min-h-0 flex-1 overflow-y-auto overscroll-contain"
      role="list"
      :aria-label="t('mail.list.list')"
      :aria-busy="props.refreshing || props.loadingOlder"
      @keydown="onKeydown"
    >
      <div
        v-for="row in rows"
        :key="row.id"
        role="listitem"
        :data-swipe-row="row.id"
        class="relative touch-pan-y overflow-hidden border-b border-border/60 [contain-intrinsic-size:auto_3.5rem] [content-visibility:auto]"
        @wheel="onRowWheel($event, row.id)"
        @pointerdown="onRowPointerDown($event, row.id)"
        @pointermove="onRowPointerMove($event, row.id)"
        @pointerup="onRowPointerUp"
        @pointercancel="onRowPointerUp"
      >
        <!--
          The swipe action sits in the space the row slides away from, so it grows with the swipe.
        -->
        <div
          v-if="swipe.id === row.id"
          :class="cn('absolute inset-y-0 right-0 flex justify-end overflow-hidden bg-destructive', !swipe.dragging && 'transition-[width] duration-200 ease-out')"
          :style="{ width: `${swipe.offset}px` }"
        >
          <button
            type="button"
            class="flex h-full min-w-[5.25rem] flex-1 flex-col items-center justify-center gap-0.5 text-xs font-medium text-white outline-none focus-visible:ring-2 focus-visible:ring-white/70 focus-visible:ring-inset"
            :tabindex="swipe.offset >= SWIPE_REVEAL_WIDTH ? 0 : -1"
            :aria-label="inTrash ? `${t('mail.list.deletePermanently')}: ${row.subject}` : `${t('mail.list.moveToTrash')}: ${row.subject}`"
            @click.stop="deleteSwipedRow(row.id)"
          >
            <Trash2Icon class="size-4" aria-hidden="true" />{{ inTrash ? 'Delete' : 'Trash' }}
          </button>
        </div>
        <button
          :data-message-id="row.id"
          :class="cn(
            'relative grid w-full grid-cols-[0.375rem_minmax(0,1fr)_auto] items-baseline gap-x-2.5 gap-y-0.5 bg-background py-2 pr-3 pl-3 text-left outline-none select-none',
            'hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset',
            isHighlighted(row.id) && 'bg-accent before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary hover:bg-accent focus-visible:bg-accent',
            swipe.id === row.id && !swipe.dragging && 'transition-transform duration-200 ease-out',
          )"
          :style="swipe.id === row.id ? { transform: `translateX(${-swipe.offset}px)` } : undefined"
          type="button"
          :aria-current="row.id === props.selectedId ? 'true' : undefined"
          :aria-pressed="multipleSelected ? props.selection.has(row.id) : undefined"
          @click="onRowClick($event, row.id)"
        >
          <span :class="cn('size-1.5 -translate-y-px self-center rounded-full', row.unread && 'bg-primary')" :aria-label="row.unread ? t('mail.list.unreadLabel') : undefined" />
          <span :class="cn('truncate', row.unread ? 'font-semibold' : 'font-medium text-foreground/85')" :title="row.senderTitle">{{ row.sender }}</span>
          <time class="text-xs text-muted-foreground tabular-nums" :title="row.dateTitle">{{ row.date }}</time>
          <span class="col-start-2 truncate text-[0.8125rem]">
            <span :class="cn(row.unread ? 'font-medium text-foreground' : 'text-foreground/85')">{{ row.subject }}</span>
            <span class="text-muted-foreground"> — {{ row.preview }}</span>
          </span>
          <PaperclipIcon v-if="row.attachments > 0" class="size-3 self-center justify-self-end text-muted-foreground" :aria-label="t(row.attachments === 1 ? 'mail.list.attachment_one' : 'mail.list.attachment_other', { count: row.attachments })" />
        </button>
      </div>

      <Alert v-if="props.error" variant="destructive" class="m-3 w-auto" role="status">
        <AlertDescription class="break-words">{{ t('mail.list.updateFailed') }}</AlertDescription>
        <AlertAction><Button variant="outline" size="xs" @click="emit('refresh')">{{ t('mail.list.retry') }}</Button></AlertAction>
      </Alert>
      <div class="flex h-10 items-center justify-center text-xs text-muted-foreground">
        <span v-if="props.loadingOlder" class="flex items-center gap-2" role="status"><Spinner class="size-3" aria-hidden="true" />{{ t('mail.list.loadingOlder') }}</span>
        <button v-else-if="props.hasOlder" type="button" class="rounded-sm px-2 py-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50" @click="emit('older')">{{ t('mail.list.loadOlder') }}</button>
        <span v-else-if="!props.hasOlder">{{ props.search ? t('mail.list.endResults') : t('mail.list.noOlder') }}</span>
      </div>
    </div>
  </section>
</template>
