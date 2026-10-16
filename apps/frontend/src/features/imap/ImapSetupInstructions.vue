<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ImapServer } from '@/api/contracts'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { imapSecurityLabel } from './security'

/**
 * Step-by-step account setup for the most common desktop mail clients.
 */
const props = defineProps<{ server: ImapServer; username: string }>()
const { t, locale } = useI18n()

const host = computed(() => props.server.host ?? '127.0.0.1')
const port = computed(() => props.server.port === null ? '—' : String(props.server.port))
const security = computed(() => { locale.value; return imapSecurityLabel(props.server.security) })
</script>

<template>
  <Tabs default-value="apple-mail" class="gap-3">
    <TabsList>
      <TabsTrigger value="apple-mail">Apple Mail</TabsTrigger>
      <TabsTrigger value="thunderbird">Thunderbird</TabsTrigger>
    </TabsList>
    <TabsContent value="apple-mail">
      <ol class="flex list-decimal flex-col gap-1.5 pl-5 text-sm text-muted-foreground">
        <li>{{ t('imap.instructions.apple.choose') }}</li>
        <li>{{ t('imap.instructions.apple.enter', { username }) }}</li>
        <li>{{ t('imap.instructions.apple.incoming', { host }) }}</li>
        <li>{{ t(security === t('imap.security.none') ? 'imap.instructions.apple.tlsOff' : 'imap.instructions.apple.tlsOn', { port }) }}</li>
        <li>{{ t('imap.instructions.noOutgoing') }}</li>
      </ol>
    </TabsContent>
    <TabsContent value="thunderbird">
      <ol class="flex list-decimal flex-col gap-1.5 pl-5 text-sm text-muted-foreground">
        <li>{{ t('imap.instructions.thunderbird.open') }}</li>
        <li>{{ t('imap.instructions.thunderbird.incoming', { host, port, security }) }}</li>
        <li>{{ t('imap.instructions.thunderbird.username', { username }) }}</li>
        <li>{{ t('imap.instructions.noOutgoingThunderbird') }}</li>
      </ol>
    </TabsContent>
  </Tabs>
  <p class="text-xs text-muted-foreground">
    {{ t('imap.instructions.expunging') }}
  </p>
</template>
