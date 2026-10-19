<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref, shallowRef, watch } from 'vue'
import { ArrowLeftIcon, CheckIcon, ChevronDownIcon, CopyIcon, DownloadIcon, FileTextIcon, MailIcon, MailOpenIcon, PaperclipIcon, RotateCwIcon, ShieldCheckIcon, Trash2Icon } from '@lucide/vue'
import type { MessageDetail, MessageFlags, MessageFlagOperation } from '@/api/contracts'
import { formatBytes, formatFullDate, mailboxName, senderInitial } from '@/utils/format'
import { cn } from '@/lib/utils'
import { Alert, AlertAction, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import type { MailFolder } from '@/features/shell/workspace'
import MoveToMenu from './MoveToMenu.vue'
import MessageFlagsMenu from './MessageFlagsMenu.vue'
import HtmlMessagePreview from './HtmlMessagePreview.vue'
import { createEmailPreviewDocument } from './previewLayout'
import { useCopy } from '@/composables/useCopy'

const props = withDefaults(defineProps<{
  message: MessageDetail | null
  loading: boolean
  error: string | null
  mobile: boolean
  downloadingAttachment?: number | null
  deleting?: boolean
  seen?: boolean
  seenBusy?: boolean
  flags?: MessageFlags
  folders?: MailFolder[]

  /**
   * The message is in Trash, so Delete removes it permanently instead of moving it to Trash.
   */
  trash?: boolean
}>(), {
  downloadingAttachment: null, deleting: false,

  /**
   * Give each message detail instance its own empty destination list by default.
   */
  folders: () => [], trash: false, seen: false
})
const { t } = useI18n()
const emit = defineEmits<{
  /**
   * Requests navigation back to the message list.
   */
  (event: 'back'): void

  /**
   * Requests another attempt to load the message.
   */
  (event: 'retry'): void

  /**
   * Requests downloading the original message.
   */
  (event: 'download'): void

  /**
   * Requests downloading the attachment at this index.
   */
  (event: 'attachment', index: number): void

  /**
   * Requests deleting the applicable message or messages.
   */
  (event: 'delete'): void

  /**
   * Requests moving the applicable messages to the destination.
   */
  (event: 'move', folderIdentifier: string | null): void

  /**
   * Requests copying the applicable messages to the destination.
   */
  (event: 'copy', folderIdentifier: string | null): void

  /**
   * Requests changing the messages’ seen state.
   */
  (event: 'set-seen', seen: boolean): void

  /**
   * Requests applying the specified flag operation.
   */
  (event: 'set-flags', operation: MessageFlagOperation, flags: MessageFlags): void
}>()

/*
 * Keep the desktop target mounted but hidden while the next message is being fetched.
 * Browser previews retain their existing unmount-on-loading behavior.
 */
const nativePreviewAvailable = typeof window !== 'undefined' && window.sandpost?.runtime === 'electron' && Boolean(window.sandpost.preview)
const retainedMessage = shallowRef(props.message)
const renderedMessage = computed(() => props.message ?? (nativePreviewAvailable && props.loading ? retainedMessage.value : null))
watch(() => [props.message, props.loading], () => {
  if (props.message) retainedMessage.value = props.message
  else if (!props.loading) retainedMessage.value = null
})

const activeTab = ref<'preview' | 'headers' | 'attachments'>('preview')
const previewMode = ref<'html' | 'text'>('html')
const metadataExpanded = ref(false)
const { copied, error: copyError, copy, reset: resetCopy } = useCopy<'headers'>(t('mail.error.copyHeaders'))
const hasHtml = computed(() => Boolean(renderedMessage.value?.html.trim()))
const formattedDate = computed(() => renderedMessage.value ? formatFullDate(renderedMessage.value.receivedAt) : '')

/**
 * Header lookups are case-insensitive, so the map is keyed by lowercase field name.
 */
const headers = computed(() => {
  const byName = new Map<string, string[]>()
  for (const [name, values] of Object.entries(renderedMessage.value?.headers ?? {})) byName.set(name.toLowerCase(), values)
  return byName
})
const sender = computed(() => header('from') || renderedMessage.value?.from.join(', ') || '—')
const recipient = computed(() => header('to') || renderedMessage.value?.to.join(', ') || '—')
const carbonCopy = computed(() => header('cc') || renderedMessage.value?.cc.join(', ') || '')
const metadata = computed(() => [
  [t('mail.detail.from'), sender.value],
  [t('mail.detail.to'), recipient.value],
  [t('mail.detail.cc'), carbonCopy.value],
  [t('mail.detail.bcc'), header('bcc')],
  [t('mail.detail.replyTo'), header('reply-to')],
  [t('mail.detail.date'), header('date') || formattedDate.value],
].filter(([, value]) => value))

/**
 * Display name and address of the first sender, for the header's primary line.
 */
const senderName = computed(() => mailboxName(renderedMessage.value?.from[0] ?? sender.value))
const senderAddress = computed(() => renderedMessage.value?.from[0] ?? '')
const headerEntries = computed(() => Object.entries(renderedMessage.value?.headers ?? {}))
const copiedHeaders = computed(() => headerEntries.value.flatMap(([name, values]) => values.map((value) => `${name}: ${value}`)).join('\n'))

/**
 * Captured HTML is wrapped once per message in a document with a restrictive policy.
 */
const htmlDocument = computed(() => hasHtml.value
  ? createEmailPreviewDocument(renderedMessage.value!.html)
  : '')

watch(() => props.message?.id, () => {
  activeTab.value = 'preview'
  previewMode.value = hasHtml.value ? 'html' : 'text'
  metadataExpanded.value = false
  resetCopy()
})

/**
 * Returns the joined values of a header field, matched without regard to case.
 */
function header(name: string): string {
  return headers.value.get(name)?.join(', ') ?? ''
}

/**
 * Switches between HTML and plain text while ignoring attempts to deselect both.
 */
function setPreviewMode(value: unknown): void {
  if (value === 'html' || value === 'text') previewMode.value = value
}

/**
 * Copies all captured headers, briefly confirming success or exposing clipboard failures.
 */
async function copyAllHeaders(): Promise<void> {
  await copy(copiedHeaders.value, 'headers')
}
</script>

<template>
  <section class="flex min-h-0 min-w-0 flex-col bg-background" :aria-label="t('mail.detail.label')">
    <template v-if="renderedMessage">
      <Empty v-if="!props.message && props.loading" role="status">
        <EmptyHeader>
          <EmptyMedia variant="icon"><Spinner /></EmptyMedia>
          <EmptyTitle>{{ t('mail.detail.opening') }}</EmptyTitle>
        </EmptyHeader>
      </Empty>
      <header v-show="props.message" class="flex h-11 shrink-0 items-center gap-1 border-b px-2 sm:px-3">
        <Button v-if="props.mobile" class="lg:hidden" variant="ghost" size="icon-sm" :aria-label="t('mail.detail.back')" @click="emit('back')"><ArrowLeftIcon /></Button>
        <span class="min-w-0 flex-1 truncate px-1 text-xs text-muted-foreground">{{ formattedDate }}</span>
        <Button variant="ghost" size="icon-sm" :disabled="props.seenBusy" :aria-label="props.seen ? t('mail.detail.markUnread') : t('mail.detail.markRead')" :title="props.seen ? t('mail.detail.markUnread') : t('mail.detail.markRead')" @click="emit('set-seen', !props.seen)">
          <MailOpenIcon v-if="props.seen" /><MailIcon v-else />
        </Button>
        <Button variant="ghost" size="sm" class="text-muted-foreground" :aria-label="t('mail.detail.downloadOriginal')" :title="t('mail.common.downloadEml')" @click="emit('download')">
          <DownloadIcon data-icon="inline-start" /><span class="max-sm:sr-only">.eml</span>
        </Button>
        <MessageFlagsMenu v-if="props.flags" :flags="[props.flags]" :disabled="props.deleting" @change="(operation, flags) => emit('set-flags', operation, flags)" />
        <MoveToMenu v-if="renderedMessage" :folders="props.folders" :disabled="props.deleting" :trash="!props.trash" @move="emit('move', $event)" @trash="emit('delete')" />
        <MoveToMenu v-if="renderedMessage" :folders="props.folders" :disabled="props.deleting" copy @copy="emit('copy', $event)" />
        <Button
          variant="ghost"
          size="icon-sm"
          class="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
          :aria-label="props.trash ? t('mail.detail.deleteEmail') : t('mail.detail.trashEmail')"
          :title="props.trash ? t('mail.list.deletePermanently') : t('mail.list.moveToTrash')"
          :disabled="props.deleting"
          @click="emit('delete')"
        >
          <Spinner v-if="props.deleting" aria-hidden="true" />
          <Trash2Icon v-else />
        </Button>
      </header>

      <Alert v-if="props.error" variant="destructive" class="mx-4 mt-3 w-auto pr-24" role="alert">
        <AlertDescription>{{ props.error }}</AlertDescription>
        <AlertAction><Button variant="outline" size="xs" @click="emit('retry')"><RotateCwIcon data-icon="inline-start" />{{ t('mail.detail.retry') }}</Button></AlertAction>
      </Alert>

      <article v-show="props.message" class="min-h-0 flex-1 overflow-y-auto">
        <div class="mx-auto flex w-full max-w-5xl flex-col gap-5 px-4 py-5 sm:px-6">
          <div class="flex flex-col gap-4">
            <h2 class="text-lg/snug font-semibold tracking-tight break-words">{{ renderedMessage.subject || t('mail.detail.noSubject') }}</h2>

            <div class="flex items-start gap-3">
              <span class="flex size-9 shrink-0 items-center justify-center rounded-full bg-muted text-sm font-medium text-muted-foreground" aria-hidden="true">{{ senderInitial(renderedMessage.from) }}</span>
              <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                <div class="flex min-w-0 flex-wrap items-baseline gap-x-2">
                  <span class="truncate font-medium">{{ senderName }}</span>
                  <span v-if="senderAddress && senderAddress !== senderName" class="truncate text-xs text-muted-foreground">{{ senderAddress }}</span>
                </div>
                <div class="flex min-w-0 items-start gap-1 text-xs text-muted-foreground">
                  <span class="min-w-0 break-all"><span class="text-muted-foreground/70">{{ t('mail.detail.to') }}</span> {{ recipient }}<template v-if="carbonCopy"> · <span class="text-muted-foreground/70">{{ t('mail.detail.cc') }}</span> {{ carbonCopy }}</template></span>
                  <button
                    class="flex shrink-0 items-center gap-0.5 rounded-sm px-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                    type="button"
                    :aria-expanded="metadataExpanded"
                    aria-controls="message-metadata"
                    @click="metadataExpanded = !metadataExpanded"
                  >
                    {{ t('mail.detail.details') }}<ChevronDownIcon :class="cn('size-3.5 transition-transform', metadataExpanded && 'rotate-180')" aria-hidden="true" />
                  </button>
                </div>
                <dl v-if="metadataExpanded" id="message-metadata" class="mt-2 grid grid-cols-[4.5rem_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-md border bg-muted/30 px-3 py-2 text-xs">
                  <template v-for="[label, value] in metadata" :key="label">
                    <dt class="text-muted-foreground">{{ label }}</dt>
                    <dd class="break-all">{{ value }}</dd>
                  </template>
                </dl>
              </div>
            </div>
          </div>

          <Tabs v-model="activeTab" class="gap-3">
            <div class="flex items-end justify-between gap-3 border-b">
              <TabsList variant="line" class="justify-start" :aria-label="t('mail.detail.inspection')">
                <TabsTrigger value="preview" class="flex-none">{{ t('mail.detail.preview') }}</TabsTrigger>
                <TabsTrigger value="headers" class="flex-none">{{ t('mail.detail.headers') }}</TabsTrigger>
                <TabsTrigger v-if="renderedMessage.attachments.length" value="attachments" class="flex-none">
                  <PaperclipIcon aria-hidden="true" />{{ t('mail.detail.attachments') }}
                  <Badge variant="secondary" class="tabular-nums">{{ renderedMessage.attachments.length }}</Badge>
                </TabsTrigger>
              </TabsList>
              <ToggleGroup v-if="hasHtml && activeTab === 'preview'" type="single" variant="outline" size="sm" class="mb-1.5" :model-value="previewMode" :aria-label="t('mail.detail.previewFormat')" @update:model-value="setPreviewMode">
                <ToggleGroupItem value="html" class="h-7 px-2 text-xs">HTML</ToggleGroupItem>
                <ToggleGroupItem value="text" class="h-7 px-2 text-xs">{{ t('mail.detail.plainText') }}</ToggleGroupItem>
              </ToggleGroup>
            </div>

            <TabsContent value="preview" class="flex flex-col gap-3">
              <div v-if="previewMode === 'html' && hasHtml" class="overflow-hidden rounded-lg border">
                <HtmlMessagePreview :document="htmlDocument" :message-identifier="renderedMessage.id" />
                <p class="flex items-center gap-1.5 border-t bg-muted/40 px-3 py-1.5 text-xs text-muted-foreground">
                  <ShieldCheckIcon class="size-3.5 text-success" aria-hidden="true" />{{ t('mail.detail.isolatedPreview') }}
                </p>
              </div>
              <pre v-else class="min-h-90 rounded-lg border bg-muted/30 p-4 font-mono text-[0.8125rem]/relaxed whitespace-pre-wrap break-words">{{ renderedMessage.text || t('mail.detail.noBody') }}</pre>
            </TabsContent>

            <TabsContent value="headers" class="flex flex-col gap-3">
              <div class="flex items-center justify-between gap-3">
                <span class="text-xs text-muted-foreground">{{ t('mail.detail.headerCount', { count: headerEntries.length }) }}</span>
                <div class="flex items-center gap-1">
                  <Button variant="ghost" size="sm" :disabled="!copiedHeaders" @click="copyAllHeaders">
                    <CheckIcon v-if="copied === 'headers'" data-icon="inline-start" class="text-success" /><CopyIcon v-else data-icon="inline-start" />{{ copied === 'headers' ? t('mail.common.copied') : t('mail.common.copyAll') }}
                  </Button>
                  <Button variant="ghost" size="sm" @click="emit('download')"><DownloadIcon data-icon="inline-start" />{{ t('mail.common.downloadEml') }}</Button>
                </div>
              </div>
              <Alert v-if="copyError" variant="destructive" role="alert"><AlertDescription>{{ copyError }}</AlertDescription></Alert>
              <dl v-if="headerEntries.length" class="divide-y overflow-hidden rounded-lg border">
                <div v-for="[name, values] in headerEntries" :key="name" class="grid gap-1 px-3 py-2 sm:grid-cols-[minmax(8rem,25%)_minmax(0,1fr)] sm:gap-4">
                  <dt class="text-xs font-medium text-muted-foreground break-all">{{ name }}</dt>
                  <dd class="font-mono text-xs/relaxed whitespace-pre-wrap break-all">{{ values.join(', ') }}</dd>
                </div>
              </dl>
              <p v-else class="py-6 text-center text-muted-foreground">{{ t('mail.detail.noHeaders') }}</p>
              <Item variant="muted" size="sm">
                <ItemContent>
                  <ItemTitle>{{ t('mail.detail.originalSource') }}</ItemTitle>
                  <ItemDescription>{{ t('mail.detail.rawDescription') }}</ItemDescription>
                </ItemContent>
                <ItemActions><Button variant="outline" size="sm" @click="emit('download')"><DownloadIcon data-icon="inline-start" />{{ t('mail.common.downloadEml') }}</Button></ItemActions>
              </Item>
            </TabsContent>

            <TabsContent v-if="renderedMessage.attachments.length" value="attachments">
              <ItemGroup class="gap-2">
                <Item v-for="(attachment, index) in renderedMessage.attachments" :key="`${attachment.filename}-${index}`" variant="outline" size="sm">
                  <ItemMedia variant="icon" class="size-8 rounded-md bg-muted"><FileTextIcon /></ItemMedia>
                  <ItemContent class="min-w-0">
                    <ItemTitle class="w-full truncate">{{ attachment.filename || t('mail.detail.unnamedAttachment') }}</ItemTitle>
                    <ItemDescription class="break-all">{{ attachment.contentType }} · {{ formatBytes(attachment.size) }}</ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button variant="outline" size="sm" :disabled="props.downloadingAttachment !== null" @click="emit('attachment', index)">
                      <Spinner v-if="props.downloadingAttachment === index" data-icon="inline-start" aria-hidden="true" />
                      <DownloadIcon v-else data-icon="inline-start" />
                      {{ props.downloadingAttachment === index ? t('mail.detail.downloading') : t('mail.common.download') }}
                    </Button>
                  </ItemActions>
                </Item>
              </ItemGroup>
            </TabsContent>
          </Tabs>

          <p class="flex items-center gap-1.5 text-xs text-muted-foreground/70"><MailOpenIcon class="size-3.5" aria-hidden="true" />{{ t('mail.detail.capturedBy') }}</p>
        </div>
      </article>
    </template>

    <Empty v-else-if="props.loading" role="status">
      <EmptyHeader>
        <EmptyMedia variant="icon"><Spinner /></EmptyMedia>
        <EmptyTitle>{{ t('mail.detail.opening') }}</EmptyTitle>
      </EmptyHeader>
    </Empty>
    <Empty v-else-if="props.error" role="alert">
      <EmptyHeader>
        <EmptyMedia variant="icon" class="bg-destructive/10 text-destructive"><RotateCwIcon /></EmptyMedia>
        <EmptyTitle>{{ t('mail.detail.openFailed') }}</EmptyTitle>
        <EmptyDescription>{{ props.error }}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent><Button variant="outline" size="sm" @click="emit('retry')">{{ t('mail.common.tryAgain') }}</Button></EmptyContent>
    </Empty>
    <Empty v-else>
      <EmptyHeader>
        <EmptyMedia variant="icon"><MailOpenIcon /></EmptyMedia>
        <EmptyTitle>{{ t('mail.detail.noSelection') }}</EmptyTitle>
        <EmptyDescription>{{ t('mail.detail.chooseMessage') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  </section>
</template>
