<script setup lang="ts">
import { MailboxIcon } from '@lucide/vue'
import { cn } from '@/lib/utils'

/**
 * `compact` sizes the mark for the application header rather than authentication pages.
 */
defineProps<{ compact?: boolean }>()
</script>

<template>
  <span :class="cn('flex items-center font-semibold tracking-tight', compact ? 'gap-2' : 'gap-2.5')">
    <span :class="cn('flex shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground', compact ? 'size-6 rounded-md' : 'size-8')" aria-hidden="true">
      <MailboxIcon :class="compact ? 'size-3.5' : 'size-4'" />
    </span>
    <span :class="compact ? 'text-sm' : 'text-base'">Sandpost</span>
  </span>
</template>
