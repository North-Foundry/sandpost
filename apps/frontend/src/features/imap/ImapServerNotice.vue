<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { CircleOffIcon, ShieldAlertIcon, TriangleAlertIcon } from '@lucide/vue'
import type { ImapServer } from '@/api/contracts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { DEFAULT_IMAP_LISTEN, usesTls } from './security'

/**
 * Warn about how this installation's IMAP service can be reached: disabled, unencrypted, or
 * encrypted with a certificate that mail clients must trust first.
 */
const props = defineProps<{ server: ImapServer }>()
const { t } = useI18n()

const tls = computed(() => props.server.enabled && usesTls(props.server.security))
</script>

<template>
  <Alert v-if="!server.enabled" variant="destructive">
    <CircleOffIcon />
    <AlertTitle>{{ t('imap.serverNotice.disabledTitle') }}</AlertTitle>
    <AlertDescription>
      <span class="block">
        {{ t('imap.serverNotice.disabledBody') }}
      </span>
      <span class="block">
        {{ t('imap.serverNotice.enableBody', { defaultAddress: DEFAULT_IMAP_LISTEN }) }}
      </span>
    </AlertDescription>
  </Alert>
  <Alert v-else-if="!tls" class="border-warning/40">
    <TriangleAlertIcon class="text-warning!" />
    <AlertTitle>{{ t('imap.serverNotice.unencryptedTitle') }}</AlertTitle>
    <AlertDescription>
      {{ t('imap.serverNotice.unencryptedBody') }}
    </AlertDescription>
  </Alert>
  <Alert v-else>
    <ShieldAlertIcon />
    <AlertTitle>{{ t('imap.serverNotice.certificateTitle') }}</AlertTitle>
    <AlertDescription>
      {{ t('imap.serverNotice.certificateBody') }}
    </AlertDescription>
  </Alert>
</template>
