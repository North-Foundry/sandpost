<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed } from 'vue'
import { RouterLink } from 'vue-router'
import BrandMark from '@/components/BrandMark.vue'
import { SidebarTrigger, useSidebar } from '@/components/ui/sidebar'
import { insetTitleBar } from '@/runtime/windowChrome'
import { cn } from '@/lib/utils'

const { t } = useI18n()

const { isMobile, state } = useSidebar()

/**
 * Without the docked sidebar the leading section keeps only the window controls and the toggle.
 */
const compact = computed(() => isMobile.value || state.value === 'collapsed')
</script>

<template>
  <!--
    Full-width bar above the sidebar and content; its leading section tracks the sidebar width.
  -->
  <header class="window-drag flex h-12 shrink-0 items-center border-b">
    <div
      :class="cn(
        'flex h-full shrink-0 items-center overflow-hidden border-r pr-3 pl-[max(0.75rem,var(--window-controls-inset))] transition-[width,background-color,border-color] duration-200 ease-linear',
        compact
          ? 'w-[calc(max(0.75rem,var(--window-controls-inset))+2.75rem)] border-transparent'
          : 'w-(--sidebar-width) border-sidebar-border bg-sidebar',
      )"
    >
      <!--
        Shrinks to nothing as the section narrows, so the toggle stays pinned to its trailing edge.
      -->
      <div class="flex min-w-0 flex-1 items-center">
        <RouterLink
          v-if="!insetTitleBar"
          :to="{ name: 'mail' }"
          :class="cn('flex shrink-0 items-center rounded-md outline-none transition-opacity duration-200 focus-visible:ring-2 focus-visible:ring-sidebar-ring', compact && 'invisible opacity-0')"
          :aria-label="t('common.brandInbox')"
        >
          <BrandMark compact />
        </RouterLink>
      </div>
      <SidebarTrigger class="size-8 shrink-0 text-muted-foreground" />
    </div>
    <div class="flex min-w-0 flex-1 items-center justify-center px-2 sm:px-3">
      <slot />
    </div>
  </header>
</template>
