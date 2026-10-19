<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRouter } from 'vue-router'
import {
  ArchiveRestoreIcon, CheckIcon, CopyIcon, MailboxIcon, MoreHorizontalIcon, PlusIcon, PowerIcon, PowerOffIcon, RefreshCwIcon, Settings2Icon,
  Trash2Icon, XIcon,
} from '@lucide/vue'
import type { ImapAccount, ImapServer, IssuedImapCredentials } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { formatFullDate, formatListDate } from '@/utils/format'
import { Alert, AlertAction, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import ImapAccountCreateDialog from '@/features/imap/ImapAccountCreateDialog.vue'
import ImapAccountDialogs from '@/features/imap/ImapAccountDialogs.vue'
import ImapServerNotice from '@/features/imap/ImapServerNotice.vue'
import { describeRestoration, useImapAccountActions } from '@/features/imap/accountActions'
import { imapSecurityLabel, mailboxCountLabel } from '@/features/imap/security'
import { useCopy } from '@/composables/useCopy'
import { useWorkspace } from '@/features/shell/workspace'

const client = useSandpostClient()
const { t } = useI18n()
const router = useRouter()
const workspace = useWorkspace()
const canMutate = computed(() => Boolean(workspace.user.value && workspace.user.value.role !== 'viewer'))
const accounts = ref<ImapAccount[]>([])
const server = ref<ImapServer | null>(null)
const loading = ref(true)
const error = ref('')
const rowError = ref('')
const createOpen = ref(false)
const toggling = ref<string | null>(null)
const { copied: copiedUsername, copy } = useCopy()

/**
 * Replace one account in the list with the server's latest representation.
 */
function replace(account: ImapAccount): void {
  accounts.value = accounts.value.map((candidate) => candidate.id === account.id ? account : candidate)
}

const actions = useImapAccountActions(client, {
  /**
   * Replace the changed account in the list.
   */
  changed: replace,

  /**
   * Remove the deleted account from the list.
   */
  deleted: (id) => { accounts.value = accounts.value.filter((account) => account.id !== id) },
})

/**
 * Load the IMAP service details and the user's accounts.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const [loadedServer, loadedAccounts] = await Promise.all([client.imap.server(), client.imap.list()])
    server.value = loadedServer
    accounts.value = loadedAccounts
  } catch (failure) {
    error.value = describeError(failure, t('imap.errors.loadAccounts'))
  } finally {
    loading.value = false
  }
}

/**
 * Show the new account's one-time credentials and add it to the list.
 */
function created(credentials: IssuedImapCredentials): void {
  accounts.value = [...accounts.value, credentials.account]
  actions.showIssued(credentials, false)
}

/**
 * Enable or disable an account in place; disabling signs its mail clients out.
 */
async function toggle(account: ImapAccount): Promise<void> {
  if (!canMutate.value || toggling.value) return
  toggling.value = account.id
  rowError.value = ''
  try {
    replace(await client.imap.update(account.id, { enabled: !account.enabled }))
  } catch (failure) {
    rowError.value = describeError(failure, t('imap.errors.updateAccount'))
  } finally {
    toggling.value = null
  }
}

/**
 * Copy a username, briefly confirming it in the open menu or reporting clipboard failures inline.
 */
async function copyUsername(account: ImapAccount): Promise<void> {
  rowError.value = ''
  if (!await copy(account.username, account.id)) rowError.value = t('imap.errors.copyUsername')
}

onMounted(load)
</script>

<template>
  <section class="h-full overflow-y-auto" aria-labelledby="imap-accounts-title">
    <div class="mx-auto flex max-w-6xl flex-col gap-6 p-4 sm:p-6">
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div class="flex flex-col gap-1">
          <h1 id="imap-accounts-title" class="text-lg font-semibold tracking-tight">{{ t('imap.accounts.title') }}</h1>
          <p class="text-sm text-muted-foreground">
            {{ t('imap.accounts.description') }}
          </p>
        </div>
        <Button v-if="canMutate" :disabled="loading || Boolean(error)" @click="createOpen = true"><PlusIcon data-icon="inline-start" />{{ t('imap.create.submit') }}</Button>
      </div>

      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('imap.common.retry') }}</Button></AlertDescription>
      </Alert>

      <div v-if="loading" class="flex flex-col gap-3">
        <Skeleton class="h-16 w-full" />
        <Skeleton v-for="index in 3" :key="index" class="h-12 w-full" />
      </div>
      <template v-else-if="server">
        <ImapServerNotice :server="server" />
        <p v-if="server.enabled" class="text-sm text-muted-foreground">
          {{ t('imap.accounts.serverConnection', { address: `${server.host}:${server.port}`, security: imapSecurityLabel(server.security) }) }}
        </p>

        <Alert v-if="actions.restoreResult.value" role="status">
          <ArchiveRestoreIcon />
          <AlertDescription>{{ describeRestoration(actions.restoreResult.value.restored, actions.restoreResult.value.accountName) }}</AlertDescription>
          <AlertAction>
            <Button variant="ghost" size="icon-xs" :aria-label="t('imap.common.dismiss')" @click="actions.restoreResult.value = null"><XIcon /></Button>
          </AlertAction>
        </Alert>
        <Alert v-if="rowError" variant="destructive" role="alert"><AlertDescription>{{ rowError }}</AlertDescription></Alert>

        <Empty v-if="!accounts.length" class="border">
          <EmptyHeader>
            <EmptyMedia variant="icon"><MailboxIcon /></EmptyMedia>
            <EmptyTitle>{{ t('imap.accounts.emptyTitle') }}</EmptyTitle>
            <EmptyDescription>{{ t('imap.accounts.emptyDescription') }}</EmptyDescription>
          </EmptyHeader>
          <EmptyContent v-if="canMutate"><Button @click="createOpen = true"><PlusIcon data-icon="inline-start" />{{ t('imap.create.submit') }}</Button></EmptyContent>
        </Empty>
        <div v-else class="overflow-hidden rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{{ t('imap.common.name') }}</TableHead>
                <TableHead class="max-sm:hidden">{{ t('imap.connection.username') }}</TableHead>
                <TableHead class="w-24">{{ t('imap.accounts.status') }}</TableHead>
                <TableHead class="max-md:hidden">{{ t('imap.accounts.mailboxes') }}</TableHead>
                <TableHead class="max-md:hidden">{{ t('imap.accounts.lastUsed') }}</TableHead>
                <TableHead v-if="canMutate" class="w-12"><span class="sr-only">{{ t('imap.common.actions') }}</span></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="account in accounts" :key="account.id">
                <TableCell class="max-w-48">
                  <RouterLink
                    :to="{ name: 'imap-account', params: { id: account.id } }"
                    class="block truncate rounded-sm font-medium underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                  >{{ account.name }}</RouterLink>
                  <span class="block truncate text-xs text-muted-foreground">{{ account.mode === 'mirror' ? t('imap.account.mirrorMode') : t('imap.account.independentMode') }}</span>
                </TableCell>
                <TableCell class="max-w-56 max-sm:hidden"><span class="block truncate font-mono text-xs">{{ account.username }}</span></TableCell>
                <TableCell v-if="canMutate">
                  <Badge :variant="account.enabled ? 'secondary' : 'outline'">{{ account.enabled ? t('imap.accounts.active') : t('imap.accounts.disabled') }}</Badge>
                </TableCell>
                <TableCell class="text-muted-foreground max-md:hidden" :title="account.mailboxes.map((mailbox) => mailbox.name).join(', ')">
                  {{ mailboxCountLabel(account) }}
                </TableCell>
                <TableCell class="text-muted-foreground max-md:hidden" :title="account.lastUsedAt ? formatFullDate(account.lastUsedAt) : undefined">
                  {{ account.lastUsedAt ? formatListDate(account.lastUsedAt) : t('imap.common.never') }}
                </TableCell>
                <TableCell>
                  <div class="flex justify-end">
                    <DropdownMenu>
                      <DropdownMenuTrigger as-child>
                        <Button variant="ghost" size="icon-sm" :aria-label="t('imap.accounts.actionsFor', { account: account.name })" :disabled="toggling === account.id">
                          <Spinner v-if="toggling === account.id" />
                          <MoreHorizontalIcon v-else />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" class="w-56">
                        <DropdownMenuGroup>
                          <DropdownMenuItem @select="router.push({ name: 'imap-account', params: { id: account.id } })"><Settings2Icon />{{ t('imap.accounts.configure') }}…</DropdownMenuItem>
                          <DropdownMenuItem @select="(event: Event) => { event.preventDefault(); void copyUsername(account) }">
                            <CheckIcon v-if="copiedUsername === account.id" class="text-success! *:text-success!" /><CopyIcon v-else />{{ copiedUsername === account.id ? t('imap.common.copied') : t('imap.accounts.copyUsername') }}
                          </DropdownMenuItem>
                          <DropdownMenuItem @select="actions.confirmRegenerate(account)"><RefreshCwIcon />{{ t('imap.dialogs.regenerate') }}…</DropdownMenuItem>
                          <DropdownMenuItem @select="actions.confirmRestore(account)"><ArchiveRestoreIcon />{{ t('imap.dialogs.restore') }}…</DropdownMenuItem>
                          <DropdownMenuItem @select="toggle(account)">
                            <component :is="account.enabled ? PowerOffIcon : PowerIcon" />{{ account.enabled ? t('imap.accounts.disable') : t('imap.accounts.enable') }}
                          </DropdownMenuItem>
                        </DropdownMenuGroup>
                        <DropdownMenuSeparator />
                        <DropdownMenuGroup>
                          <DropdownMenuItem variant="destructive" @select="actions.confirmDelete(account)"><Trash2Icon />{{ t('imap.common.delete') }}…</DropdownMenuItem>
                        </DropdownMenuGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>
      </template>
    </div>

    <ImapAccountCreateDialog v-if="canMutate" v-model:open="createOpen" @created="created" />
    <ImapAccountDialogs v-if="canMutate" :actions="actions" />
  </section>
</template>
