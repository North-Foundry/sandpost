<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ChevronRightIcon, PlugZapIcon, TriangleAlertIcon } from '@lucide/vue'
import type { OutboundConnectionTest, OutboundPolicy, OutboundSecurity, SmtpOutbound } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel, FieldLegend, FieldSet } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Separator } from '@/components/ui/separator'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import BusyDialog from '@/components/BusyDialog.vue'
import OutboundTestResult from '@/features/outbound/OutboundTestResult.vue'
import {
  OUTBOUND_DEFAULTS, emptyOutboundForm, hasAdvancedSettings, outboundFormFrom, outboundFormProblems, outboundFormWarnings,
  outboundInputFromForm, portAfterSecurityChange, type OutboundForm, type OutboundFormField,
} from './outboundForm'

/**
 * Create or edit an SMTP Outbound and test the typed settings before saving.
 *
 * The stored password is never shown: a blank password field keeps it, and a draft test of an
 * existing outbound passes its identifier so the server reuses the stored password.
 */
const open = defineModel<boolean>('open', { required: true })
const props = defineProps<{
  /**
   * The outbound being edited, or null to create one.
   */
  outbound: SmtpOutbound | null
  policy: OutboundPolicy | null
}>()
const emit = defineEmits<{
  /**
   * Report the saved outbound.
   */
  (event: 'saved', outbound: SmtpOutbound): void
}>()
const client = useSandpostClient()
const { t } = useI18n()

const form = ref<OutboundForm>(emptyOutboundForm())
const advancedOpen = ref(false)
const submitted = ref(false)
const saving = ref(false)
const saveError = ref('')
const testing = ref(false)
const testResult = ref<OutboundConnectionTest | null>(null)
const testError = ref('')

const editing = computed(() => props.outbound !== null)
const passwordConfigured = computed(() => Boolean(props.outbound?.passwordConfigured) && props.outbound?.authenticationType !== 'none')
const problems = computed(() => outboundFormProblems(form.value, {
  passwordConfigured: passwordConfigured.value,
  credentialsAvailable: props.policy?.credentialsAvailable,
}))
const warnings = computed(() => outboundFormWarnings(form.value))
const valid = computed(() => Object.keys(problems.value).length === 0)
const busy = computed(() => saving.value || testing.value)

/**
 * Problems are shown after the first save or test attempt, and immediately for the allowlist.
 */
function problem(field: OutboundFormField): string {
  const found = problems.value[field]
  if (!found || (!submitted.value && field !== 'allowedRecipients')) return ''
  return t(found.key, found.params ?? {})
}

/**
 * Move the port along with the security mode while it still has the previous mode's default.
 */
function changeSecurity(value: unknown): void {
  const next = (value === 'none' || value === 'tls' ? value : 'starttls') as OutboundSecurity
  form.value.port = portAfterSecurityChange(form.value.port, form.value.security, next)
  form.value.security = next
}

/**
 * Test the typed settings without saving; an existing outbound's stored password is reused when blank.
 */
async function test(): Promise<void> {
  submitted.value = true
  if (!valid.value || busy.value) return
  testing.value = true
  testError.value = ''
  testResult.value = null
  try {
    testResult.value = await client.admin.outbound.testDraft(outboundInputFromForm(form.value), props.outbound?.id ?? null)
  } catch (failure) {
    testError.value = describeError(failure, t('outbound.test.error'))
  } finally {
    testing.value = false
  }
}

/**
 * Create or replace the outbound; a blank password keeps the stored one.
 */
async function save(): Promise<void> {
  submitted.value = true
  if (!valid.value || busy.value) return
  saving.value = true
  saveError.value = ''
  try {
    const input = outboundInputFromForm(form.value)
    const saved = props.outbound
      ? await client.admin.outbound.update(props.outbound.id, input)
      : await client.admin.outbound.create(input)
    emit('saved', saved)
    open.value = false
  } catch (failure) {
    saveError.value = describeError(failure, t('outbound.form.saveError'))
  } finally {
    saving.value = false
  }
}

/*
 * Reset the form whenever the dialog opens, so a previous draft or password never lingers.
 */
watch(open, (isOpen) => {
  if (!isOpen) {
    submitted.value = false
    form.value.password = ''
    return
  }
  form.value = props.outbound ? outboundFormFrom(props.outbound) : emptyOutboundForm()
  advancedOpen.value = hasAdvancedSettings(form.value)
  submitted.value = false
  saveError.value = ''
  testError.value = ''
  testResult.value = null
}, { immediate: true })

/*
 * A previous test result no longer describes edited settings.
 */
watch(form, () => { testResult.value = null; testError.value = '' }, { deep: true })
</script>

<template>
  <BusyDialog
    v-model:open="open"
    :title="editing ? t('outbound.form.editTitle', { name: props.outbound?.name ?? '' }) : t('outbound.form.createTitle')"
    :description="t('outbound.form.description')"
    :busy="busy"
    content-class="sm:max-w-2xl"
  >
    <form class="flex flex-col gap-5" novalidate @submit.prevent="save">
      <FieldGroup class="gap-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <Field :data-invalid="Boolean(problem('name'))" :data-disabled="busy">
            <FieldLabel for="outbound-name">{{ t('outbound.form.name') }}</FieldLabel>
            <Input id="outbound-name" v-model="form.name" :placeholder="t('outbound.form.namePlaceholder')" maxlength="100" autocomplete="off" :aria-invalid="Boolean(problem('name'))" :disabled="busy" />
            <FieldError v-if="problem('name')">{{ problem('name') }}</FieldError>
          </Field>
          <Field orientation="horizontal" class="self-end" :data-disabled="busy">
            <Switch id="outbound-enabled" v-model="form.enabled" :disabled="busy" />
            <FieldContent>
              <FieldLabel for="outbound-enabled" class="font-normal">{{ t('outbound.form.enabled') }}</FieldLabel>
              <FieldDescription class="text-xs">{{ t('outbound.form.enabledDescription') }}</FieldDescription>
            </FieldContent>
          </Field>
        </div>
        <div class="grid gap-4 sm:grid-cols-[minmax(0,1fr)_7rem]">
          <Field :data-invalid="Boolean(problem('host'))" :data-disabled="busy">
            <FieldLabel for="outbound-host">{{ t('outbound.form.host') }}</FieldLabel>
            <Input id="outbound-host" v-model="form.host" class="font-mono" placeholder="smtp.example.com" autocomplete="off" spellcheck="false" :aria-invalid="Boolean(problem('host'))" :disabled="busy" />
            <FieldError v-if="problem('host')">{{ problem('host') }}</FieldError>
          </Field>
          <Field :data-invalid="Boolean(problem('port'))" :data-disabled="busy">
            <FieldLabel for="outbound-port">{{ t('outbound.form.port') }}</FieldLabel>
            <Input id="outbound-port" v-model="form.port" class="font-mono" inputmode="numeric" autocomplete="off" :aria-invalid="Boolean(problem('port'))" :disabled="busy" />
            <FieldError v-if="problem('port')">{{ problem('port') }}</FieldError>
          </Field>
        </div>
        <div class="grid gap-4 sm:grid-cols-2">
          <Field :data-disabled="busy">
            <FieldLabel for="outbound-security">{{ t('outbound.form.security') }}</FieldLabel>
            <NativeSelect id="outbound-security" :model-value="form.security" class="w-full" :disabled="busy" @update:model-value="changeSecurity">
              <NativeSelectOption value="starttls">{{ t('outbound.security.starttls') }}</NativeSelectOption>
              <NativeSelectOption value="tls">{{ t('outbound.security.tls') }}</NativeSelectOption>
              <NativeSelectOption value="none">{{ t('outbound.security.none') }}</NativeSelectOption>
            </NativeSelect>
          </Field>
          <Field :data-disabled="busy">
            <FieldLabel for="outbound-authentication">{{ t('outbound.form.authentication') }}</FieldLabel>
            <NativeSelect id="outbound-authentication" v-model="form.authenticationType" class="w-full" :disabled="busy">
              <NativeSelectOption value="plain">PLAIN</NativeSelectOption>
              <NativeSelectOption value="login">LOGIN</NativeSelectOption>
              <NativeSelectOption value="none">{{ t('outbound.authentication.none') }}</NativeSelectOption>
            </NativeSelect>
          </Field>
        </div>
        <div v-if="form.authenticationType !== 'none'" class="grid gap-4 sm:grid-cols-2">
          <Field :data-invalid="Boolean(problem('username'))" :data-disabled="busy">
            <FieldLabel for="outbound-username">{{ t('outbound.form.username') }}</FieldLabel>
            <Input id="outbound-username" v-model="form.username" class="font-mono" autocomplete="off" spellcheck="false" :aria-invalid="Boolean(problem('username'))" :disabled="busy" />
            <FieldError v-if="problem('username')">{{ problem('username') }}</FieldError>
          </Field>
          <Field :data-invalid="Boolean(problem('password'))" :data-disabled="busy">
            <FieldLabel for="outbound-password">{{ t('outbound.form.password') }}</FieldLabel>
            <Input
              id="outbound-password"
              v-model="form.password"
              type="password"
              autocomplete="new-password"
              :placeholder="passwordConfigured ? t('outbound.form.passwordKeep') : ''"
              :aria-invalid="Boolean(problem('password'))"
              :disabled="busy"
            />
            <FieldError v-if="problem('password')">{{ problem('password') }}</FieldError>
          </Field>
        </div>
        <Alert v-if="props.policy && !props.policy.credentialsAvailable && form.authenticationType !== 'none'" variant="destructive">
          <TriangleAlertIcon />
          <AlertDescription>{{ t('outbound.policy.credentialsUnavailable') }}</AlertDescription>
        </Alert>
        <Alert v-if="warnings.plaintextCredentials" class="border-warning/40">
          <TriangleAlertIcon class="text-warning!" />
          <AlertTitle>{{ t('outbound.form.plaintextTitle') }}</AlertTitle>
          <AlertDescription>{{ t('outbound.form.plaintextDescription') }}</AlertDescription>
        </Alert>
      </FieldGroup>

      <Separator />

      <FieldSet class="gap-3">
        <FieldLegend variant="label">{{ t('outbound.form.allowedRecipients') }}</FieldLegend>
        <FieldDescription class="-mt-1">{{ t('outbound.form.allowedRecipientsDescription') }}</FieldDescription>
        <Field :data-invalid="Boolean(problem('allowedRecipients'))" :data-disabled="busy">
          <FieldLabel for="outbound-allowed-recipients" class="sr-only">{{ t('outbound.form.allowedRecipients') }}</FieldLabel>
          <Textarea
            id="outbound-allowed-recipients"
            v-model="form.allowedRecipients"
            class="min-h-20 font-mono"
            placeholder="*@example.com&#10;qa@company.test"
            spellcheck="false"
            :aria-invalid="Boolean(problem('allowedRecipients'))"
            :disabled="busy"
          />
          <FieldError v-if="problem('allowedRecipients')">{{ problem('allowedRecipients') }}</FieldError>
        </Field>
        <Alert v-if="warnings.nobodyAllowed" class="border-warning/40">
          <TriangleAlertIcon class="text-warning!" />
          <AlertTitle>{{ t('outbound.form.nobodyTitle') }}</AlertTitle>
          <AlertDescription>{{ t('outbound.form.nobodyDescription') }}</AlertDescription>
        </Alert>
        <Alert v-else-if="warnings.everyoneAllowed" class="border-warning/40">
          <TriangleAlertIcon class="text-warning!" />
          <AlertDescription>{{ t('outbound.form.everyoneDescription') }}</AlertDescription>
        </Alert>
      </FieldSet>

      <Collapsible v-model:open="advancedOpen" class="flex flex-col gap-4">
        <CollapsibleTrigger class="flex w-fit items-center gap-1.5 rounded-md text-sm font-medium outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
          <ChevronRightIcon :class="cn('size-4 transition-transform', advancedOpen && 'rotate-90')" aria-hidden="true" />
          {{ t('outbound.form.advanced') }}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <FieldGroup class="gap-4">
            <div class="grid gap-4 sm:grid-cols-2">
              <Field :data-invalid="Boolean(problem('overrideFrom'))" :data-disabled="busy">
                <FieldLabel for="outbound-override-from">{{ t('outbound.form.overrideFrom') }}</FieldLabel>
                <Input id="outbound-override-from" v-model="form.overrideFrom" class="font-mono" placeholder="noreply@example.com" autocomplete="off" spellcheck="false" :aria-invalid="Boolean(problem('overrideFrom'))" :disabled="busy" />
                <FieldDescription class="text-xs">{{ t('outbound.form.overrideFromDescription') }}</FieldDescription>
                <FieldError v-if="problem('overrideFrom')">{{ problem('overrideFrom') }}</FieldError>
              </Field>
              <Field :data-invalid="Boolean(problem('envelopeFrom'))" :data-disabled="busy">
                <FieldLabel for="outbound-envelope-from">{{ t('outbound.form.envelopeFrom') }}</FieldLabel>
                <Input id="outbound-envelope-from" v-model="form.envelopeFrom" class="font-mono" placeholder="bounces@example.com" autocomplete="off" spellcheck="false" :aria-invalid="Boolean(problem('envelopeFrom'))" :disabled="busy" />
                <FieldDescription class="text-xs">{{ t('outbound.form.envelopeFromDescription') }}</FieldDescription>
                <FieldError v-if="problem('envelopeFrom')">{{ problem('envelopeFrom') }}</FieldError>
              </Field>
            </div>
            <div class="grid gap-4 sm:grid-cols-3">
              <Field :data-invalid="Boolean(problem('connectTimeout'))" :data-disabled="busy">
                <FieldLabel for="outbound-connect-timeout">{{ t('outbound.form.connectTimeout') }}</FieldLabel>
                <Input id="outbound-connect-timeout" v-model="form.connectTimeout" inputmode="numeric" :placeholder="String(OUTBOUND_DEFAULTS.connectTimeout)" :aria-invalid="Boolean(problem('connectTimeout'))" :disabled="busy" />
                <FieldError v-if="problem('connectTimeout')">{{ problem('connectTimeout') }}</FieldError>
              </Field>
              <Field :data-invalid="Boolean(problem('smtpTimeout'))" :data-disabled="busy">
                <FieldLabel for="outbound-smtp-timeout">{{ t('outbound.form.smtpTimeout') }}</FieldLabel>
                <Input id="outbound-smtp-timeout" v-model="form.smtpTimeout" inputmode="numeric" :placeholder="String(OUTBOUND_DEFAULTS.smtpTimeout)" :aria-invalid="Boolean(problem('smtpTimeout'))" :disabled="busy" />
                <FieldError v-if="problem('smtpTimeout')">{{ problem('smtpTimeout') }}</FieldError>
              </Field>
              <Field :data-invalid="Boolean(problem('maxConnections'))" :data-disabled="busy">
                <FieldLabel for="outbound-max-connections">{{ t('outbound.form.maxConnections') }}</FieldLabel>
                <Input id="outbound-max-connections" v-model="form.maxConnections" inputmode="numeric" :placeholder="String(OUTBOUND_DEFAULTS.maxConnections)" :aria-invalid="Boolean(problem('maxConnections'))" :disabled="busy" />
                <FieldError v-if="problem('maxConnections')">{{ problem('maxConnections') }}</FieldError>
              </Field>
            </div>
            <Field :data-invalid="Boolean(problem('heloName'))" :data-disabled="busy">
              <FieldLabel for="outbound-helo-name">{{ t('outbound.form.heloName') }}</FieldLabel>
              <Input id="outbound-helo-name" v-model="form.heloName" class="font-mono" placeholder="sandpost.example.com" autocomplete="off" spellcheck="false" :aria-invalid="Boolean(problem('heloName'))" :disabled="busy" />
              <FieldDescription class="text-xs">{{ t('outbound.form.heloNameDescription') }}</FieldDescription>
              <FieldError v-if="problem('heloName')">{{ problem('heloName') }}</FieldError>
            </Field>
            <Field orientation="horizontal" :data-disabled="busy || form.security === 'none'">
              <Checkbox
                id="outbound-tls-verify"
                :model-value="form.tlsVerify"
                :disabled="busy || form.security === 'none'"
                @update:model-value="form.tlsVerify = $event === true"
              />
              <FieldContent>
                <FieldLabel for="outbound-tls-verify" class="font-normal">{{ t('outbound.form.tlsVerify') }}</FieldLabel>
                <FieldDescription class="text-xs">{{ t('outbound.form.tlsVerifyDescription') }}</FieldDescription>
              </FieldContent>
            </Field>
            <Alert v-if="warnings.certificateUnverified" class="border-warning/40">
              <TriangleAlertIcon class="text-warning!" />
              <AlertDescription>{{ t('outbound.form.tlsVerifyOff') }}</AlertDescription>
            </Alert>
          </FieldGroup>
        </CollapsibleContent>
      </Collapsible>

      <OutboundTestResult v-if="testResult" :result="testResult" />
      <Alert v-if="testError" variant="destructive" role="alert"><AlertDescription>{{ testError }}</AlertDescription></Alert>
      <Alert v-if="saveError" variant="destructive" role="alert"><AlertDescription>{{ saveError }}</AlertDescription></Alert>

      <DialogFooter class="sm:justify-between">
        <Button variant="outline" type="button" :disabled="busy" @click="test">
          <Spinner v-if="testing" data-icon="inline-start" aria-hidden="true" /><PlugZapIcon v-else data-icon="inline-start" />
          {{ testing ? t('outbound.test.testing') : t('outbound.test.action') }}
        </Button>
        <div class="flex flex-col-reverse gap-2 sm:flex-row">
          <Button variant="outline" type="button" :disabled="busy" @click="open = false">{{ t('outbound.common.cancel') }}</Button>
          <Button type="submit" :disabled="busy || (submitted && !valid)">
            <Spinner v-if="saving" data-icon="inline-start" aria-hidden="true" />
            {{ saving ? t('outbound.common.saving') : editing ? t('outbound.common.save') : t('outbound.form.create') }}
          </Button>
        </div>
      </DialogFooter>
    </form>
  </BusyDialog>
</template>
