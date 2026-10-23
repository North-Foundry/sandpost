<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { CircleCheckIcon, CircleXIcon } from '@lucide/vue'
import type { OutboundConnectionTest } from '@/api/contracts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { formatDuration } from '@/features/personas/format'

/**
 * Show the outcome of an SMTP Outbound connection test: where it connected, whether TLS and
 * authentication succeeded, and the failure category, SMTP code and server text.
 */
const props = defineProps<{ result: OutboundConnectionTest }>()
const { t, locale } = useI18n()

const rows = computed(() => {
  const result = props.result
  return [
    { label: t('outbound.test.address'), value: result.address ?? t('outbound.test.notConnected') },
    { label: t('outbound.test.encryption'), value: result.encrypted ? t('outbound.test.encrypted') : t('outbound.test.notEncrypted') },
    { label: t('outbound.test.authentication'), value: result.authenticated ? t('outbound.test.authenticated') : t('outbound.test.notAuthenticated') },
    { label: t('outbound.test.duration'), value: formatDuration(result.durationMilliseconds, locale.value) },
  ]
})
</script>

<template>
  <Alert :variant="props.result.success ? 'default' : 'destructive'" role="status">
    <CircleCheckIcon v-if="props.result.success" class="text-success!" />
    <CircleXIcon v-else />
    <AlertTitle>{{ props.result.success ? t('outbound.test.success') : t('outbound.test.failed') }}</AlertTitle>
    <AlertDescription class="flex flex-col gap-2">
      <p v-if="props.result.error" class="break-words">
        <span class="font-medium">{{ t(`outbound.errorKind.${props.result.error.kind}`) }}</span>
        <template v-if="props.result.error.code !== null"> · {{ t('outbound.test.code', { code: props.result.error.code }) }}</template>
        <span v-if="props.result.error.message" class="block font-mono text-xs">{{ props.result.error.message }}</span>
      </p>
      <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
        <template v-for="row in rows" :key="row.label">
          <dt class="text-muted-foreground">{{ row.label }}</dt>
          <dd class="truncate font-mono">{{ row.value }}</dd>
        </template>
        <template v-if="props.result.extensions.length">
          <dt class="text-muted-foreground">{{ t('outbound.test.extensions') }}</dt>
          <dd class="font-mono break-words">{{ props.result.extensions.join(' · ') }}</dd>
        </template>
      </dl>
    </AlertDescription>
  </Alert>
</template>
