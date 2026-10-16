<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { FieldError } from '@/components/ui/field'
import { Spinner } from '@/components/ui/spinner'
import BusyDialog from '@/components/BusyDialog.vue'
import type { ImapAccountActions } from './accountActions'
import ImapCredentialsDialog from './ImapCredentialsDialog.vue'

/**
 * Confirmation dialogs and the one-time credentials dialog for IMAP account actions.
 */
const props = defineProps<{ actions: ImapAccountActions }>()
const { t } = useI18n()
const {
  issued, regenerated, regenerateTarget, regenerating, regenerateError, restoreTarget, restoring, restoreError, deleteTarget, deleting,
  deleteError,
} = props.actions
</script>

<template>
  <BusyDialog
    :open="Boolean(regenerateTarget)"
    :title="t('imap.dialogs.regenerateTitle', { account: regenerateTarget?.name ?? t('imap.common.thisAccount') })"
    :description="t('imap.dialogs.regenerateDescription')"
    :busy="regenerating"
    @update:open="!$event && (regenerateTarget = null)"
  >
    <FieldError v-if="regenerateError" role="alert">{{ regenerateError }}</FieldError>
    <DialogFooter>
      <Button variant="outline" :disabled="regenerating" @click="regenerateTarget = null">{{ t('imap.common.cancel') }}</Button>
      <Button :disabled="regenerating" @click="actions.regenerate()">
        <Spinner v-if="regenerating" data-icon="inline-start" aria-hidden="true" />{{ regenerating ? t('imap.dialogs.regenerating') : t('imap.dialogs.regenerate') }}
      </Button>
    </DialogFooter>
  </BusyDialog>

  <BusyDialog
    :open="Boolean(restoreTarget)"
    :title="t('imap.dialogs.restoreTitle', { account: restoreTarget?.name ?? t('imap.common.thisAccount') })"
    :description="t('imap.dialogs.restoreDescription')"
    :busy="restoring"
    @update:open="!$event && (restoreTarget = null)"
  >
    <FieldError v-if="restoreError" role="alert">{{ restoreError }}</FieldError>
    <DialogFooter>
      <Button variant="outline" :disabled="restoring" @click="restoreTarget = null">{{ t('imap.common.cancel') }}</Button>
      <Button :disabled="restoring" @click="actions.restore()">
        <Spinner v-if="restoring" data-icon="inline-start" aria-hidden="true" />{{ restoring ? t('imap.dialogs.restoring') : t('imap.dialogs.restore') }}
      </Button>
    </DialogFooter>
  </BusyDialog>

  <BusyDialog
    :open="Boolean(deleteTarget)"
    :title="t('imap.dialogs.deleteTitle', { account: deleteTarget?.name ?? t('imap.common.imapAccount') })"
    :description="t('imap.dialogs.deleteDescription')"
    :busy="deleting"
    @update:open="!$event && (deleteTarget = null)"
  >
    <FieldError v-if="deleteError" role="alert">{{ deleteError }}</FieldError>
    <DialogFooter>
      <Button variant="outline" :disabled="deleting" @click="deleteTarget = null">{{ t('imap.common.cancel') }}</Button>
      <Button variant="destructive" :disabled="deleting" @click="actions.remove()">
        <Spinner v-if="deleting" data-icon="inline-start" aria-hidden="true" />{{ deleting ? t('imap.common.deleting') : t('imap.dialogs.deleteAccount') }}
      </Button>
    </DialogFooter>
  </BusyDialog>

  <ImapCredentialsDialog :credentials="issued" :regenerated="regenerated" @closed="actions.credentialsClosed()" />
</template>
