<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { CodeXmlIcon, GripHorizontalIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { Skeleton } from '@/components/ui/skeleton'
import type { SandpostEmailPreviewBridge } from '@/api/desktop'
import { useTheme } from '@/composables/useTheme'
import { constrainInspectorHeight, visiblePreviewRectangle } from './previewLayout'

const props = defineProps<{ document: string; messageIdentifier: string }>()
const { t } = useI18n()
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
let resizeObserver: ResizeObserver | undefined

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
  resizeObserver?.disconnect()
  stopSubscription?.()
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
        <iframe
          class="block size-full border-0 bg-white [color-scheme:light]"
          :style="{ visibility: nativeVisible ? 'hidden' : 'visible', pointerEvents: dragging ? 'none' : undefined }"
          :srcdoc="props.document"
          sandbox=""
          :title="t('mail.preview.title')"
          referrerpolicy="no-referrer"
        />
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
