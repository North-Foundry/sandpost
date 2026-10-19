<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { cn } from '@/lib/utils'
import type { PreviewRectangle } from './previewLayout'
import { TONE_DOTS, type LinkResultDescription } from './linkCheck'

const props = defineProps<{
  /**
   * The hovered or focused link's result, or null to hide the card.
   */
  description: LinkResultDescription | null

  /**
   * The link's box in viewport coordinates.
   */
  anchor: PreviewRectangle | null
}>()

/**
 * Space between the link and the card, and between the card and the viewport edge.
 */
const GAP = 6
const MARGIN = 8

const card = ref<HTMLElement>()
const position = ref({ top: -9999, left: -9999 })

/**
 * Place the card below the link, or above it when the viewport has no room below, and keep it
 * horizontally inside the viewport. The card never covers the link it describes.
 */
async function place(): Promise<void> {
  await nextTick()
  const anchor = props.anchor
  const element = card.value
  if (!anchor || !element) return
  const { width, height } = element.getBoundingClientRect()
  const below = anchor.y + anchor.height + GAP
  const top = below + height + MARGIN <= window.innerHeight || anchor.y - GAP - height < MARGIN
    ? below
    : anchor.y - GAP - height
  const left = Math.max(MARGIN, Math.min(anchor.x, window.innerWidth - width - MARGIN))
  position.value = { top, left }
}

watch(() => [props.description, props.anchor?.x, props.anchor?.y, props.anchor?.width, props.anchor?.height], () => {
  void place()
}, { immediate: true })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.description && props.anchor"
      ref="card"
      role="tooltip"
      data-slot="link-check-card"
      class="pointer-events-none fixed z-50 flex w-72 max-w-[calc(100vw-1rem)] flex-col gap-1 rounded-lg bg-popover p-2.5 text-sm text-popover-foreground shadow-md ring-1 ring-foreground/10 animate-in fade-in-0 zoom-in-95 duration-100"
      :style="{ top: `${position.top}px`, left: `${position.left}px` }"
    >
      <p class="flex items-center gap-1.5 font-medium">
        <span :class="cn('size-1.5 shrink-0 rounded-full', TONE_DOTS[props.description.tone])" aria-hidden="true" />{{ props.description.title }}
      </p>
      <p class="text-xs font-medium tabular-nums">{{ props.description.status }}</p>
      <p class="text-xs text-muted-foreground">{{ props.description.explanation }}</p>
      <p v-if="props.description.destination" class="text-xs break-all text-muted-foreground">{{ props.description.destination }}</p>
      <p class="text-xs text-muted-foreground">{{ props.description.checked }} · {{ props.description.checkedAbsolute }}</p>
    </div>
  </Teleport>
  <span class="sr-only" role="status" aria-live="polite">{{ props.description ? `${props.description.title}. ${props.description.status}.` : '' }}</span>
</template>
