<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { CodeXmlIcon, GripHorizontalIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { Skeleton } from '@/components/ui/skeleton'
import type { LinkCheckResult } from '@/api/contracts'
import type { SandpostEmailPreviewBridge } from '@/api/desktop'
import { useTheme } from '@/composables/useTheme'
import { formatFullDate } from '@/utils/format'
import LinkCheckHoverCard from './LinkCheckHoverCard.vue'
import { describeLinkResult, normalizeLinkHref } from './linkCheck'
import { constrainInspectorHeight, visiblePreviewRectangle, type PreviewRectangle } from './previewLayout'

const props = withDefaults(defineProps<{
  document: string
  messageIdentifier: string

  /**
   * Stored link check results by normalized URL; hovering a link with a result describes it.
   */
  linkResults?: ReadonlyMap<string, LinkCheckResult>

  /**
   * Current time in milliseconds, so relative check times stay fresh.
   */
  now?: number
}>(), {
  /**
   * Without results no link is described.
   */
  linkResults: () => new Map(),
  now: () => Date.now(),
})
const { t, locale } = useI18n()
const { isDark } = useTheme()
const inspectorTheme = computed(() => isDark.value ? 'dark' as const : 'light' as const)
const bridge: SandpostEmailPreviewBridge | undefined = typeof window !== 'undefined' && window.sandpost?.runtime === 'electron'
  ? window.sandpost.preview : undefined
const region = ref<HTMLElement>()
const previewRegion = ref<HTMLElement>()
const inspectorRegion = ref<HTMLElement>()
const inspectorOpen = ref(false)
const inspectorReady = ref(false)
const inspectorVisible = ref(false)
const nativeReady = ref(false)
const nativeVisible = ref(false)
const nativeFailed = ref(false)
const previewError = ref('')
const dragging = ref(false)
const availableHeight = ref(420)
const maximumNativeHeight = ref<number>()
const inspectorRatio = ref(0.35)
const inspectorHeight = computed(() => constrainInspectorHeight(inspectorRatio.value * availableHeight.value, availableHeight.value - 8))
let previewIdentifier = ''
let revision = 0
let inspectorOperation = 0
let animationFrame = 0
let lastLayout = ''
let stopped = false
let stopSubscription: (() => void) | undefined
let stopLinkSubscription: (() => void) | undefined
let resizeObserver: ResizeObserver | undefined
const frame = shallowRef<HTMLIFrameElement>()

/**
 * How often the browser preview's hovered or focused link is read, and how long a link must stay
 * targeted before its result appears.
 */
const LINK_POLL_INTERVAL = 100
const LINK_SHOW_DELAY = 250
const hoveredLink = shallowRef<{ url: string; anchor: PreviewRectangle } | null>(null)
let linkCandidate: { url: string; since: number } | null = null
let linkTimer: ReturnType<typeof setInterval> | undefined

/**
 * Describe the targeted link's stored result for the browser hover card.
 */
const hoveredDescription = computed(() => {
  const result = hoveredLink.value ? props.linkResults.get(hoveredLink.value.url) : undefined
  return result ? describeLinkResult(result, t, props.now, locale.value, formatFullDate) : null
})

/**
 * Find the link under the pointer, or the keyboard-focused link, in the browser preview.
 *
 * The preview document is same-origin but runs no script: the application reads its hover and
 * focus state and geometry from outside, without adding listeners, elements or styles to it.
 * Only links with a stored result are reported, with their box in viewport coordinates.
 */
function targetedPreviewLink(): { url: string; anchor: PreviewRectangle } | null {
  const element = frame.value
  let preview: Document | null = null
  try {
    preview = element?.contentDocument ?? null
  } catch {
    return null
  }
  if (!element || !preview || nativeVisible.value) return null
  const hovered = preview.querySelectorAll(':hover')
  let link = hovered[hovered.length - 1]?.closest('a[href], area[href]') ?? null
  if (!link && document.activeElement === element) {
    const focused = preview.activeElement
    if (focused?.matches('a[href]:focus-visible, area[href]:focus-visible')) link = focused
  }
  const url = normalizeLinkHref(link?.getAttribute('href'))
  if (!link || !url || !props.linkResults.has(url)) return null
  const frameBounds = element.getBoundingClientRect()
  const bounds = link.getBoundingClientRect()
  const anchor = {
    x: frameBounds.x + element.clientLeft + bounds.x,
    y: frameBounds.y + element.clientTop + bounds.y,
    width: bounds.width,
    height: bounds.height,
  }
  const visible = bounds.width > 0 && bounds.height > 0
    && anchor.y + anchor.height > frameBounds.y && anchor.y < frameBounds.bottom
  return visible ? { url, anchor } : null
}

/**
 * Follow the targeted link, showing its result after a short delay and switching immediately
 * between links once a result is visible.
 */
function trackPreviewLink(): void {
  const targeted = targetedPreviewLink()
  if (!targeted) {
    linkCandidate = null
    hoveredLink.value = null
    return
  }
  if (hoveredLink.value) {
    hoveredLink.value = targeted
    return
  }
  if (linkCandidate?.url !== targeted.url) linkCandidate = { url: targeted.url, since: Date.now() }
  else if (Date.now() - linkCandidate.since >= LINK_SHOW_DELAY) hoveredLink.value = targeted
}

/**
 * Read the browser preview's targeted link only while some link has a stored result.
 */
watch(() => props.linkResults.size > 0, (active) => {
  if (linkTimer !== undefined) clearInterval(linkTimer)
  linkTimer = undefined
  hoveredLink.value = null
  linkCandidate = null
  if (active && !stopped) linkTimer = setInterval(trackPreviewLink, LINK_POLL_INTERVAL)
}, { immediate: true })

/**
 * Show or hide the native popover for a link targeted in the native preview.
 */
function describeNativeLink(event: { id: string; url: string | null }): void {
  if (!bridge?.linkPopover || event.id !== previewIdentifier || stopped) return
  const url = normalizeLinkHref(event.url)
  const result = url ? props.linkResults.get(url) : undefined
  const description = result ? describeLinkResult(result, t, props.now, locale.value, formatFullDate) : null
  void bridge.linkPopover({
    id: event.id,
    popover: description && {
      tone: description.tone,
      title: description.title,
      status: description.status,
      explanation: description.explanation,
      destination: description.destination,
      checked: `${description.checked} · ${description.checkedAbsolute}`,
    },
  }).catch(() => { })
}

/**
 * Fall back to the browser sandbox if the native target cannot be loaded.
 */
function failPreview(error: unknown): void {
  nativeFailed.value = true
  nativeReady.value = false
  nativeVisible.value = false
  inspectorOpen.value = false
  inspectorReady.value = false
  inspectorVisible.value = false
  previewError.value = error instanceof Error ? error.message : String(error)
  void bridge?.release(previewIdentifier).catch(() => { })
}

/**
 * Load the same document in a stable native target, ignoring obsolete load completions.
 */
watch(() => [props.messageIdentifier, props.document], async () => {
  if (!bridge || stopped || nativeFailed.value) return
  const identifier = `${props.messageIdentifier}:${++revision}:${crypto.randomUUID()}`
  previewIdentifier = identifier
  nativeReady.value = false
  nativeVisible.value = false
  lastLayout = ''
  try {
    await bridge.show({ id: identifier, html: props.document })
    if (stopped || previewIdentifier !== identifier) return
    nativeReady.value = true
    lastLayout = ''
  } catch (error) {
    if (!stopped && previewIdentifier === identifier) failPreview(error)
  }
}, { immediate: true, flush: 'sync' })

/**
 * Native children sit above DOM content, so hide them while application overlays are present.
 */
function hasApplicationOverlay(): boolean {
  return [...document.querySelectorAll<HTMLElement>('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [data-slot="popover-content"], [data-slot="sheet-overlay"], [data-slot="dialog-overlay"], [data-slot="alert-dialog-overlay"]')]
    .some((element) => element.dataset.state !== 'closed' && element.getClientRects().length > 0 && getComputedStyle(element).visibility !== 'hidden')
}

/**
 * Return bounds only for a wholly visible region; the iframe renders any clipped preview.
 */
function visibleBounds(element: HTMLElement | undefined) {
  if (!element || document.hidden || dragging.value || hasApplicationOverlay()) return null
  const bounds = element.getBoundingClientRect()
  if (!element.getClientRects().length) return null
  const clipping = [{ x: 0, y: 0, width: window.innerWidth, height: window.innerHeight }]
  for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
    const style = getComputedStyle(ancestor)
    if (style.visibility === 'hidden' || style.display === 'none') return null
    if (ancestor !== element && /(auto|scroll|hidden|clip)/.test(`${style.overflowX} ${style.overflowY}`)) {
      const rectangle = ancestor.getBoundingClientRect()
      clipping.push({ x: rectangle.x + ancestor.clientLeft, y: rectangle.y + ancestor.clientTop, width: ancestor.clientWidth, height: ancestor.clientHeight })
    }
  }
  return visiblePreviewRectangle({ x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height }, clipping)
}

/**
 * Follow geometry and the resolved app theme, sending IPC only when either changes.
 */
function synchronizeLayout(): void {
  if (stopped) return
  if (bridge && !nativeFailed.value && previewIdentifier) {
    const viewer = region.value?.closest?.('article')
    if (viewer) maximumNativeHeight.value = Math.max(0, viewer.clientHeight - 48)
    const preview = visibleBounds(previewRegion.value)
    /*
     * Keep the inspector and its actual target visible together.
     */
    const devtools = preview && inspectorOpen.value ? visibleBounds(inspectorRegion.value) : null
    nativeVisible.value = Boolean(preview && nativeReady.value)
    inspectorVisible.value = Boolean(devtools && nativeReady.value && inspectorReady.value)
    const layout = { id: previewIdentifier, preview, devtools, theme: inspectorTheme.value }
    const serialized = JSON.stringify(layout)
    if (serialized !== lastLayout) {
      lastLayout = serialized
      const identifier = previewIdentifier
      void bridge.layout(layout).catch((error: unknown) => {
        if (!stopped && previewIdentifier === identifier) failPreview(error)
      })
    }
  }
  animationFrame = requestAnimationFrame(synchronizeLayout)
}

/**
 * Toggle the native inspector, reserving its DOM layout before exposing the native view.
 */
async function toggleInspector(): Promise<void> {
  if (!bridge || !nativeReady.value) return
  const operation = ++inspectorOperation
  const identifier = previewIdentifier
  const open = !inspectorOpen.value
  inspectorOpen.value = open
  inspectorReady.value = false
  inspectorVisible.value = false
  await nextTick()
  if (stopped || operation !== inspectorOperation || identifier !== previewIdentifier) return
  region.value?.scrollIntoView({ block: 'nearest' })
  lastLayout = ''
  try {
    await bridge.layout({ id: identifier, preview: visibleBounds(previewRegion.value), devtools: open ? visibleBounds(inspectorRegion.value) : null, theme: inspectorTheme.value })
    if (stopped || operation !== inspectorOperation || identifier !== previewIdentifier) return
    await bridge.inspect({ id: identifier, open })
  }
  catch (error) {
    if (!stopped && operation === inspectorOperation && identifier === previewIdentifier) failPreview(error)
  }
}

/**
 * Reserve pointer input for Vue while the divider crosses native surfaces.
 */
function startResize(event: PointerEvent): void {
  if (event.button !== 0) return
  dragging.value = true
    ; (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  lastLayout = ''
  if (bridge) void bridge.layout({ id: previewIdentifier, preview: null, devtools: null, theme: inspectorTheme.value }).catch(failPreview)
  event.preventDefault()
}

/**
 * Clamp the split to the available region and both panels' minimum sizes.
 */
function moveResize(event: PointerEvent): void {
  if (!dragging.value || !region.value) return
  const bounds = region.value.getBoundingClientRect()
  inspectorRatio.value = constrainInspectorHeight(bounds.bottom - event.clientY, availableHeight.value - 8) / availableHeight.value
}

/**
 * Restore native surfaces after releasing the drag capture.
 */
function finishResize(): void {
  dragging.value = false
  lastLayout = ''
}

/**
 * Provide the same bounded resize operation for keyboard users.
 */
function resizeWithKeyboard(event: KeyboardEvent): void {
  const offsets: Record<string, number> = { ArrowUp: 20, ArrowDown: -20, Home: -availableHeight.value, End: availableHeight.value }
  if (!(event.key in offsets)) return
  event.preventDefault()
  inspectorRatio.value = constrainInspectorHeight(inspectorHeight.value + offsets[event.key]!, availableHeight.value - 8) / availableHeight.value
}

onMounted(() => {
  if (!bridge) return
  stopLinkSubscription = bridge.subscribeLinks?.(describeNativeLink)
  stopSubscription = bridge.subscribe((state) => {
    if (state.id !== previewIdentifier || stopped) return
    if (state.error) { failPreview(state.error); return }
    inspectorOpen.value = state.open
    inspectorReady.value = state.open && state.ready === true
    lastLayout = ''
  })
  resizeObserver = new ResizeObserver(() => {
    availableHeight.value = region.value?.clientHeight ?? 420
    lastLayout = ''
  })
  if (region.value) resizeObserver.observe(region.value)
  animationFrame = requestAnimationFrame(synchronizeLayout)
  window.addEventListener('resize', invalidateLayout)
  window.addEventListener('pointermove', moveResize)
  window.addEventListener('pointerup', finishResize)
  window.addEventListener('pointercancel', finishResize)
  window.addEventListener('blur', finishResize)
  document.addEventListener('visibilitychange', invalidateLayout)
})

/**
 * Force bounds delivery after a host window state change, even if its dimensions match.
 */
function invalidateLayout(): void {
  lastLayout = ''
}

onBeforeUnmount(() => {
  stopped = true
  cancelAnimationFrame(animationFrame)
  if (linkTimer !== undefined) clearInterval(linkTimer)
  resizeObserver?.disconnect()
  stopSubscription?.()
  stopLinkSubscription?.()
  window.removeEventListener('resize', invalidateLayout)
  window.removeEventListener('pointermove', moveResize)
  window.removeEventListener('pointerup', finishResize)
  window.removeEventListener('pointercancel', finishResize)
  window.removeEventListener('blur', finishResize)
  document.removeEventListener('visibilitychange', invalidateLayout)
  if (bridge && previewIdentifier) void bridge.release(previewIdentifier).catch(() => { })
})
</script>

<template>
  <div>
    <div v-if="bridge" class="flex items-center justify-end gap-2 border-b bg-muted/40 px-2 py-1">
      <Button variant="ghost" size="sm" :disabled="!nativeReady || nativeFailed" :aria-pressed="inspectorOpen" :aria-label="t('mail.preview.inspectEmail')" @click="toggleInspector">
        <CodeXmlIcon data-icon="inline-start" />{{ inspectorOpen ? t('mail.preview.closeInspector') : t('mail.preview.inspect') }}
      </Button>
    </div>
    <div ref="region" class="flex h-[max(420px,calc(100dvh-340px))] flex-col overflow-hidden" :style="{ maxHeight: maximumNativeHeight === undefined ? undefined : `${maximumNativeHeight}px` }">
      <div ref="previewRegion" class="relative min-h-0 flex-1 bg-white [color-scheme:light]">
        <!--
          Same-origin without allow-scripts: the email still runs no script, while the application
          can read hover and focus state from outside to describe checked links.
        -->
        <iframe
          ref="frame"
          class="block size-full border-0 bg-white [color-scheme:light]"
          :style="{ visibility: nativeVisible ? 'hidden' : 'visible', pointerEvents: dragging ? 'none' : undefined }"
          :srcdoc="props.document"
          sandbox="allow-same-origin"
          :title="t('mail.preview.title')"
          referrerpolicy="no-referrer"
        />
        <LinkCheckHoverCard :description="hoveredDescription" :anchor="hoveredLink?.anchor ?? null" />
      </div>
      <template v-if="bridge && inspectorOpen">
        <div
          class="flex h-2 shrink-0 cursor-row-resize touch-none items-center justify-center bg-border outline-none focus-visible:ring-2 focus-visible:ring-ring"
          role="separator"
          tabindex="0"
          :aria-label="t('mail.preview.resize')"
          aria-orientation="horizontal"
          :aria-valuenow="Math.round(inspectorHeight)"
          :aria-valuemin="Math.round(constrainInspectorHeight(0, availableHeight - 8))"
          :aria-valuemax="Math.round(constrainInspectorHeight(availableHeight, availableHeight - 8))"
          @pointerdown="startResize"
          @lostpointercapture="finishResize"
          @keydown="resizeWithKeyboard"
        ><GripHorizontalIcon class="size-3 text-muted-foreground" aria-hidden="true" /></div>
        <div ref="inspectorRegion" class="shrink-0 overflow-hidden bg-muted/30" :style="{ height: `${inspectorHeight}px` }" :aria-busy="!inspectorReady || !nativeReady" :aria-label="t('mail.preview.nativeTools')">
          <div v-if="!inspectorReady || !nativeReady" class="relative flex h-full items-center justify-center overflow-hidden" role="status" aria-live="polite" :aria-label="t('mail.preview.opening')">
            <div class="absolute inset-0 bg-radial from-primary/5 via-transparent to-transparent" aria-hidden="true" />
            <div class="relative flex flex-col items-center gap-3 px-4 py-3">
              <div class="relative flex size-12 items-center justify-center" aria-hidden="true">
                <Spinner class="absolute size-14 text-primary/50 [animation-duration:1.8s]" aria-hidden="true" />
                <div class="flex size-8 items-center justify-center rounded-lg border bg-background shadow-sm">
                  <CodeXmlIcon class="size-4 text-foreground" />
                </div>
              </div>
              <span class="text-xs font-medium text-foreground">{{ t('mail.preview.opening') }}</span>
              <div class="flex items-center gap-1.5" aria-hidden="true">
                <Skeleton class="h-1 w-8 [animation-delay:-.4s]" />
                <Skeleton class="h-1 w-5 [animation-delay:-.2s]" />
                <Skeleton class="h-1 w-3" />
              </div>
            </div>
          </div>
          <p v-else-if="!inspectorVisible && !dragging" class="p-3 text-xs text-muted-foreground">{{ t('mail.preview.scrollHint') }}</p>
        </div>
      </template>
    </div>
    <p v-if="previewError" role="status" class="px-3 py-2 text-xs text-muted-foreground">{{ t('mail.preview.unavailable', { error: previewError }) }}</p>
  </div>
</template>
