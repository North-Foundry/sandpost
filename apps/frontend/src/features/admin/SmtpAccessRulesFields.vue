<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed } from 'vue'
import type { SmtpMechanism } from '@/api/contracts'
import { SMTP_MECHANISMS } from '@/api/admin'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldContent, FieldDescription, FieldError, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'

const props = defineProps<{
  /**
   * Prefix for element ids, so two instances never collide.
   */
  idPrefix: string
  disabled?: boolean

  /**
   * Whether the server can offer TLS at all; without it a TLS-only access cannot sign in.
   */
  encryptionAvailable: boolean
}>()
const requiresEncryption = defineModel<boolean>('requiresEncryption', { required: true })
const { t } = useI18n()
const mechanisms = defineModel<SmtpMechanism[]>('mechanisms', { required: true })

const MECHANISM_DETAILS: Record<SmtpMechanism, { label: string; description: string }> = {
  plain: { label: 'PLAIN', description: 'admin.smtp.mechanism.plain' },
  login: { label: 'LOGIN', description: 'admin.smtp.mechanism.login' },
}

const noMechanism = computed(() => mechanisms.value.length === 0)

/**
 * Add or remove one mechanism, keeping the canonical order.
 */
function setMechanism(mechanism: SmtpMechanism, allowed: boolean): void {
  const next = new Set(mechanisms.value)
  if (allowed) next.add(mechanism)
  else next.delete(mechanism)
  mechanisms.value = SMTP_MECHANISMS.filter((candidate) => next.has(candidate))
}
</script>

<template>
  <FieldSet class="gap-3">
    <FieldLegend variant="label">{{ t('admin.smtp.auth') }}</FieldLegend>
    <Field orientation="horizontal" :data-disabled="props.disabled">
      <Checkbox
        :id="`${props.idPrefix}-requires-encryption`"
        :model-value="requiresEncryption"
        :disabled="props.disabled"
        @update:model-value="requiresEncryption = $event === true"
      />
      <FieldContent>
        <FieldLabel :for="`${props.idPrefix}-requires-encryption`" class="font-normal">{{ t('admin.smtp.requireTls') }}</FieldLabel>
        <FieldDescription class="text-xs">
          {{ t('admin.smtp.tlsHint') }}
        </FieldDescription>
      </FieldContent>
    </Field>
    <Alert v-if="requiresEncryption && !props.encryptionAvailable" variant="destructive">
      <AlertDescription>
        {{ t('admin.smtp.noCertificate') }}
      </AlertDescription>
    </Alert>
    <Field v-for="mechanism in SMTP_MECHANISMS" :key="mechanism" orientation="horizontal" :data-disabled="props.disabled">
      <Checkbox
        :id="`${props.idPrefix}-mechanism-${mechanism}`"
        :model-value="mechanisms.includes(mechanism)"
        :disabled="props.disabled"
        @update:model-value="setMechanism(mechanism, $event === true)"
      />
      <FieldContent>
        <FieldLabel :for="`${props.idPrefix}-mechanism-${mechanism}`" class="font-normal">{{ t('admin.smtp.allowMechanism', { mechanism: MECHANISM_DETAILS[mechanism].label }) }}</FieldLabel>
        <FieldDescription class="text-xs">{{ t(MECHANISM_DETAILS[mechanism].description) }}</FieldDescription>
      </FieldContent>
    </Field>
    <FieldError v-if="noMechanism">{{ t('admin.smtp.allowAtLeastOne') }}</FieldError>
  </FieldSet>
</template>
