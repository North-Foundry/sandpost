<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  InfoIcon, MoreHorizontalIcon, PencilIcon, PlugZapIcon, PlusIcon, SendIcon, ShieldAlertIcon, Trash2Icon, TriangleAlertIcon,
} from '@lucide/vue'
import type { OutboundConnectionTest, OutboundPolicy, SmtpOutbound } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { FieldError } from '@/components/ui/field'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import BusyDialog from '@/components/BusyDialog.vue'
import OutboundTestResult from '@/features/outbound/OutboundTestResult.vue'
import { allowlistCoverage } from '@/features/outbound/recipients'
import { useAdminContext } from './context'
import OutboundDeliveries from './OutboundDeliveries.vue'
import SmtpOutboundDialog from './SmtpOutboundDialog.vue'

const client = useSandpostClient()
const { t } = useI18n()
const { isGlobalAdmin } = useAdminContext()

const outbounds = ref<SmtpOutbound[]>([])
const policy = ref<OutboundPolicy | null>(null)
const loading = ref(true)
const error = ref('')
const rowError = ref('')
const toggling = ref<string | null>(null)

const dialogOpen = ref(false)
const editTarget = ref<SmtpOutbound | null>(null)

/**
 * The outbound whose stored configuration is being tested, with the result once it arrives.
 */
const testTarget = ref<SmtpOutbound | null>(null)
const testResult = ref<OutboundConnectionTest | null>(null)
const testError = ref('')
const testing = ref(false)

const deleteTarget = ref<SmtpOutbound | null>(null)
const deleting = ref(false)
const deleteError = ref('')

/**
 * Load the policy and every outbound.
 */
async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const [loadedPolicy, loadedOutbounds] = await Promise.all([client.admin.outbound.policy(), client.admin.outbound.list()])
    policy.value = loadedPolicy
    outbounds.value = loadedOutbounds
  } catch (failure) {
    error.value = describeError(failure, t('outbound.page.loadError'))
  } finally {
    loading.value = false
  }
}

/**
 * Open the dialog to create an outbound, or to edit one.
 */
function openDialog(outbound: SmtpOutbound | null): void {
  editTarget.value = outbound
  dialogOpen.value = true
}

/**
 * Put the saved outbound in place, or add it, keeping the list ordered by name.
 */
function saved(outbound: SmtpOutbound): void {
  const others = outbounds.value.filter((candidate) => candidate.id !== outbound.id)
  outbounds.value = [...others, outbound].sort((first, second) => first.name.localeCompare(second.name))
}

/**
 * Enable or disable an outbound in place.
 */
async function toggle(outbound: SmtpOutbound, enabled: boolean): Promise<void> {
  if (toggling.value) return
  toggling.value = outbound.id
  rowError.value = ''
  try {
    saved(await client.admin.outbound.setEnabled(outbound.id, enabled))
  } catch (failure) {
    rowError.value = describeError(failure, t('outbound.page.updateError'))
  } finally {
    toggling.value = null
  }
}

/**
 * Test the stored configuration of an outbound and show the result.
 */
async function test(outbound: SmtpOutbound): Promise<void> {
  testTarget.value = outbound
  testResult.value = null
  testError.value = ''
  testing.value = true
  try {
    const result = await client.admin.outbound.test(outbound.id)
    if (testTarget.value?.id === outbound.id) testResult.value = result
  } catch (failure) {
    if (testTarget.value?.id === outbound.id) testError.value = describeError(failure, t('outbound.test.error'))
  } finally {
    testing.value = false
  }
}

/**
 * Delete the confirmed outbound; the server refuses while routing actions still use it.
 */
async function remove(): Promise<void> {
  const target = deleteTarget.value
  if (!target || deleting.value) return
  deleting.value = true
  deleteError.value = ''
  try {
    await client.admin.outbound.delete(target.id)
    outbounds.value = outbounds.value.filter((outbound) => outbound.id !== target.id)
    deleteTarget.value = null
  } catch (failure) {
    deleteError.value = describeError(failure, t('outbound.page.deleteError'))
  } finally {
    deleting.value = false
  }
}

/**
 * Summarize the recipient allowlist for the table.
 */
function recipientsSummary(outbound: SmtpOutbound): string {
  switch (allowlistCoverage(outbound.allowedRecipients)) {
    case 'nobody': return t('outbound.recipients.nobody')
    case 'everyone': return t('outbound.recipients.everyone')
    default: return t('outbound.recipients.patterns', { count: outbound.allowedRecipients.length }, outbound.allowedRecipients.length)
  }
}

onMounted(() => { if (isGlobalAdmin.value) void load() })
</script>

<template>
  <div class="mx-auto flex max-w-6xl flex-col gap-6 p-4 sm:p-6">
    <div class="flex flex-wrap items-end justify-between gap-3">
      <div class="flex flex-col gap-1">
        <h1 class="text-lg font-semibold tracking-tight">{{ t('outbound.page.title') }}</h1>
        <p class="text-sm text-muted-foreground">{{ t('outbound.page.description') }}</p>
      </div>
      <Button v-if="isGlobalAdmin" :disabled="loading || Boolean(error)" @click="openDialog(null)"><PlusIcon data-icon="inline-start" />{{ t('outbound.page.create') }}</Button>
    </div>

    <Empty v-if="!isGlobalAdmin" class="border">
      <EmptyHeader>
        <EmptyMedia variant="icon"><SendIcon /></EmptyMedia>
        <EmptyTitle>{{ t('outbound.page.onlyOwners') }}</EmptyTitle>
        <EmptyDescription>{{ t('outbound.page.ownersManage') }}</EmptyDescription>
      </EmptyHeader>
    </Empty>
    <template v-else>
      <Alert v-if="error" variant="destructive" role="alert">
        <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('outbound.common.retry') }}</Button></AlertDescription>
      </Alert>

      <div v-if="loading" class="flex flex-col gap-3">
        <Skeleton v-for="index in 3" :key="index" class="h-12 w-full" />
      </div>
      <template v-else-if="!error">
        <Alert v-if="policy && !policy.credentialsAvailable" class="border-warning/40">
          <TriangleAlertIcon class="text-warning!" />
          <AlertTitle>{{ t('outbound.policy.credentialsTitle') }}</AlertTitle>
          <AlertDescription>{{ t('outbound.policy.credentialsUnavailable') }}</AlertDescription>
        </Alert>
        <p v-if="policy" class="-mt-3 flex items-start gap-1.5 text-xs text-muted-foreground">
          <InfoIcon class="mt-px size-3.5 shrink-0" aria-hidden="true" />
          <span>
            {{ t('outbound.policy.retries', { attempts: policy.maximumAttempts, seconds: policy.retryDelaySeconds }) }}
            <template v-if="!policy.allowInternalAddresses"> {{ t('outbound.policy.internalRefused') }}</template>
          </span>
        </p>

        <Alert v-if="rowError" variant="destructive" role="alert"><AlertDescription>{{ rowError }}</AlertDescription></Alert>
        <Empty v-if="!outbounds.length" class="border">
          <EmptyHeader>
            <EmptyMedia variant="icon"><SendIcon /></EmptyMedia>
            <EmptyTitle>{{ t('outbound.page.empty') }}</EmptyTitle>
            <EmptyDescription>{{ t('outbound.page.emptyDescription') }}</EmptyDescription>
          </EmptyHeader>
          <EmptyContent><Button @click="openDialog(null)"><PlusIcon data-icon="inline-start" />{{ t('outbound.page.create') }}</Button></EmptyContent>
        </Empty>
        <div v-else class="overflow-hidden rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{{ t('outbound.form.name') }}</TableHead>
                <TableHead class="max-md:hidden">{{ t('outbound.page.server') }}</TableHead>
                <TableHead class="max-lg:hidden">{{ t('outbound.page.connection') }}</TableHead>
                <TableHead class="max-sm:hidden">{{ t('outbound.form.allowedRecipients') }}</TableHead>
                <TableHead class="max-lg:hidden">{{ t('outbound.page.routingActions') }}</TableHead>
                <TableHead class="w-20">{{ t('outbound.form.enabled') }}</TableHead>
                <TableHead class="w-12"><span class="sr-only">{{ t('outbound.page.actions') }}</span></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="outbound in outbounds" :key="outbound.id">
                <TableCell class="max-w-48">
                  <span class="block truncate font-medium">{{ outbound.name }}</span>
                  <span class="block truncate font-mono text-xs text-muted-foreground md:hidden">{{ outbound.host }}:{{ outbound.port }}</span>
                </TableCell>
                <TableCell class="max-w-56 max-md:hidden"><span class="block truncate font-mono text-xs">{{ outbound.host }}:{{ outbound.port }}</span></TableCell>
                <TableCell class="max-lg:hidden">
                  <span class="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                    <ShieldAlertIcon v-if="outbound.security === 'none' && outbound.authenticationType !== 'none'" class="size-3.5 text-warning" :aria-label="t('outbound.form.plaintextTitle')" />
                    {{ t(`outbound.security.${outbound.security}`) }} · {{ outbound.authenticationType === 'none' ? t('outbound.authentication.none') : outbound.authenticationType.toUpperCase() }}
                  </span>
                </TableCell>
                <TableCell class="max-sm:hidden" :title="outbound.allowedRecipients.join('\n') || undefined">
                  <Badge v-if="!outbound.allowedRecipients.length" variant="outline" class="border-warning/50 font-normal"><TriangleAlertIcon data-icon="inline-start" class="text-warning" />{{ recipientsSummary(outbound) }}</Badge>
                  <span v-else class="text-xs text-muted-foreground">{{ recipientsSummary(outbound) }}</span>
                </TableCell>
                <TableCell class="text-xs text-muted-foreground tabular-nums max-lg:hidden">{{ outbound.routingActionCount }}</TableCell>
                <TableCell>
                  <Switch
                    :model-value="outbound.enabled"
                    :disabled="toggling === outbound.id"
                    :aria-label="t('outbound.page.toggle', { name: outbound.name })"
                    @update:model-value="toggle(outbound, $event)"
                  />
                </TableCell>
                <TableCell>
                  <div class="flex justify-end">
                    <DropdownMenu>
                      <DropdownMenuTrigger as-child>
                        <Button variant="ghost" size="icon-sm" :aria-label="t('outbound.page.nameActions', { name: outbound.name })"><MoreHorizontalIcon /></Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" class="w-48">
                        <DropdownMenuGroup>
                          <DropdownMenuItem @select="openDialog(outbound)"><PencilIcon />{{ t('outbound.page.edit') }}</DropdownMenuItem>
                          <DropdownMenuItem @select="test(outbound)"><PlugZapIcon />{{ t('outbound.test.action') }}</DropdownMenuItem>
                        </DropdownMenuGroup>
                        <DropdownMenuSeparator />
                        <DropdownMenuGroup>
                          <DropdownMenuItem variant="destructive" @select="deleteError = ''; deleteTarget = outbound"><Trash2Icon />{{ t('outbound.page.delete') }}</DropdownMenuItem>
                        </DropdownMenuGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>

        <OutboundDeliveries />
      </template>
    </template>

    <SmtpOutboundDialog v-model:open="dialogOpen" :outbound="editTarget" :policy="policy" @saved="saved" />

    <BusyDialog
      :open="Boolean(testTarget)"
      :title="t('outbound.test.title', { name: testTarget?.name ?? '' })"
      :description="t('outbound.test.description')"
      :busy="testing"
      content-class="sm:max-w-lg"
      @update:open="!$event && (testTarget = null)"
    >
      <div v-if="testing" class="flex items-center gap-2 text-sm text-muted-foreground" role="status"><Spinner aria-hidden="true" />{{ t('outbound.test.testing') }}</div>
      <OutboundTestResult v-else-if="testResult" :result="testResult" />
      <FieldError v-if="testError" role="alert">{{ testError }}</FieldError>
      <DialogFooter>
        <Button variant="outline" :disabled="testing" @click="testTarget = null">{{ t('outbound.common.close') }}</Button>
        <Button :disabled="testing || !testTarget" @click="testTarget && test(testTarget)">
          <PlugZapIcon data-icon="inline-start" />{{ t('outbound.test.again') }}
        </Button>
      </DialogFooter>
    </BusyDialog>

    <BusyDialog
      :open="Boolean(deleteTarget)"
      :title="t('outbound.page.deleteTitle', { name: deleteTarget?.name ?? '' })"
      :description="t('outbound.page.deleteDescription')"
      :busy="deleting"
      @update:open="!$event && (deleteTarget = null)"
    >
      <Alert v-if="deleteError" variant="destructive" role="alert"><AlertDescription>{{ deleteError }}</AlertDescription></Alert>
      <DialogFooter>
        <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('outbound.common.cancel') }}</Button>
        <Button variant="destructive" :disabled="deleting" @click="remove">
          <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('outbound.common.deleting') : t('outbound.page.delete') }}
        </Button>
      </DialogFooter>
    </BusyDialog>
  </div>
</template>
