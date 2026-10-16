<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SmtpServer } from '@/api/contracts'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'

const props = defineProps<{ server: SmtpServer }>()
const { t } = useI18n()

const presentation = computed(() => {
  const listener = props.server.listener
  if (!props.server.listenAddress && listener?.source !== 'configuration') return { label: t('admin.listener.disabled'), dot: 'bg-muted-foreground/40', variant: 'outline' as const }
  if (!listener) return { label: t('admin.listener.saved'), dot: 'bg-muted-foreground/40', variant: 'outline' as const }
  if (listener.state === 'listening') return { label: t('admin.listener.listening'), dot: 'bg-success', variant: 'outline' as const }
  if (listener.state === 'failed') return { label: t('admin.listener.notListening'), dot: 'bg-destructive', variant: 'destructive' as const }
  return { label: t('admin.listener.disabled'), dot: 'bg-muted-foreground/40', variant: 'outline' as const }
})
</script>

<template>
  <Badge :variant="presentation.variant">
    <span :class="cn('size-1.5 rounded-full', presentation.dot)" aria-hidden="true" />
    {{ presentation.label }}
  </Badge>
</template>
