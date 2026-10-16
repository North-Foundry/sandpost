<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { InstanceStatus, SetupInput } from '@/api/contracts'
import { useRouter } from 'vue-router'
import { useSandpostClient } from '@/runtime/client'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import AuthLayout from '@/layouts/AuthLayout.vue'
import SetupKeyForm from '@/features/setup/SetupKeyForm.vue'
import CreateOwnerForm from '@/features/setup/CreateOwnerForm.vue'

const client = useSandpostClient()
const router = useRouter()
const { t } = useI18n()
const screen = ref<'loading' | 'unavailable' | 'key' | 'owner' | 'leaving'>('loading')
const setupKey = ref('')
const authorization = ref('')
const owner = ref<Omit<SetupInput, 'bootstrapSecret'> & { confirmation: string }>({
  name: '', email: '', password: '', confirmation: '',
})
const error = ref('')
const keyError = ref('')
const fieldErrors = ref<Partial<Record<'name' | 'email' | 'password' | 'confirmation', string>>>({})
const submitting = ref(false)
let active = true

/**
 * Focus the first input of a newly rendered form, or its invalid field.
 */
async function focusField(identifier: string): Promise<void> {
  await nextTick()
  if (!active) return
  document.getElementById(identifier)?.focus()
}

/**
 * Discard credentials as soon as setup finishes or the page is destroyed.
 */
function clearCredentials(): void {
  setupKey.value = ''
  authorization.value = ''
  owner.value.password = ''
  owner.value.confirmation = ''
}

/**
 * Let the existing route guard choose login or the authenticated application.
 */
async function leaveSetup(): Promise<void> {
  if (!active) return
  clearCredentials()
  screen.value = 'leaving'
  await router.replace({ name: 'login' })
}

/**
 * Render only the form permitted by the current backend/client status.
 */
async function showStatus(status: InstanceStatus): Promise<void> {
  if (!active) return
  if (status.initialized) return leaveSetup()
  screen.value = status.setupKeyRequired !== false ? 'key' : 'owner'
  await focusField(screen.value === 'key' ? 'setup-key' : 'owner-name')
}

/**
 * Fetch the authoritative state on entry and on an explicit connection retry.
 */
async function loadStatus(): Promise<void> {
  screen.value = 'loading'
  error.value = ''
  clearCredentials()
  try {
    await showStatus(await client.instanceStatus())
  } catch {
    if (active) screen.value = 'unavailable'
  }
}

/**
 * Translate only recognized public errors; never render internal backend messages.
 */
async function handleFailure(failure: unknown): Promise<void> {
  if (!active) return
  const metadata = failure as { status?: number; code?: unknown; message?: unknown } | null
  const code = typeof metadata?.code === 'string' ? metadata.code : ''
  const message = typeof metadata?.message === 'string' ? metadata.message : ''
  if (code === 'already_initialized' || /already[ _]?initialized/i.test(message)) {
    await leaveSetup()
    return
  }

  let status: InstanceStatus
  try {
    status = await client.instanceStatus()
  } catch {
    if (!active) return
    error.value = t('auth.setup.error.cannotReach')
    return
  }
  if (!active) return
  if (status.initialized) return leaveSetup()

  if (code === 'authentication_failed' || metadata?.status === 401 || /authentication required|Invalid bootstrap secret/i.test(message)) {
    authorization.value = ''
    setupKey.value = ''
    if (status.setupKeyRequired !== false) {
      screen.value = 'key'
      keyError.value = t('auth.setup.error.invalidKey')
    } else {
      error.value = t('auth.setup.error.unauthorized')
    }
    return
  }

  const validation = [
    ['name', 'name must contain 1 to 100 printable characters', 'auth.setup.error.invalidName'],
    ['email', 'enter a valid email address', 'auth.setup.error.invalidEmail'],
    ['password', 'password must contain 12 to 1024 bytes', 'auth.setup.error.invalidPassword'],
  ] as const
  for (const [field, backendMessage, readableMessage] of validation) {
    if (code === backendMessage || message.includes(backendMessage)) {
      fieldErrors.value[field] = t(readableMessage)
      return
    }
  }
  error.value = t('auth.setup.error.failed')
}

/**
 * Exchange the boot key for temporary authorization and immediately clear the key.
 */
async function submitKey(): Promise<void> {
  if (submitting.value) return
  error.value = ''
  keyError.value = ''
  if (!setupKey.value.trim()) {
    keyError.value = t('auth.setup.error.enterKey')
    await focusField('setup-key')
    return
  }
  submitting.value = true
  try {
    const result = await client.authorizeSetup({ setupKey: setupKey.value.trim() })
    if (!active) return
    setupKey.value = ''
    authorization.value = result.authorization
    screen.value = 'owner'
  } catch (failure) {
    await handleFailure(failure)
  } finally {
    submitting.value = false
    if (screen.value === 'owner') await focusField('owner-name')
    else if (screen.value === 'key') await focusField('setup-key')
  }
}

/**
 * Create the sole first owner using temporary authorization or the trusted client.
 */
async function submitOwner(): Promise<void> {
  if (submitting.value) return
  error.value = ''
  fieldErrors.value = {}
  if (owner.value.password !== owner.value.confirmation) {
    fieldErrors.value.confirmation = t('auth.setup.error.passwordMismatch')
    await focusField('owner-confirmation')
    return
  }
  submitting.value = true
  try {
    await client.setup({
      bootstrapSecret: authorization.value,
      name: owner.value.name.trim(), email: owner.value.email.trim(), password: owner.value.password,
    })
    await leaveSetup()
  } catch (failure) {
    await handleFailure(failure)
  } finally {
    submitting.value = false
    if (screen.value === 'key') await focusField('setup-key')
    else {
      const invalidField = Object.keys(fieldErrors.value)[0]
      if (invalidField) await focusField(`owner-${invalidField}`)
    }
  }
}

onMounted(loadStatus)
onUnmounted(() => {
  active = false
  clearCredentials()
})
</script>

<template>
  <AuthLayout :label="t('auth.setup.ariaLabel')">
    <div v-if="screen === 'loading' || screen === 'leaving'" class="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground" role="status" aria-live="polite">
      <Spinner aria-hidden="true" />
      {{ screen === 'loading' ? t('auth.setup.checkingInstance') : t('auth.setup.opening') }}
    </div>
    <div v-else-if="screen === 'unavailable'" class="flex flex-col gap-5">
      <Alert>
        <AlertTitle>{{ t('auth.setup.error.cannotReachTitle') }}</AlertTitle>
        <AlertDescription>{{ t('auth.setup.error.checkServer') }}</AlertDescription>
      </Alert>
      <Button class="w-full" size="lg" type="button" @click="loadStatus">{{ t('auth.common.tryAgain') }}</Button>
    </div>
    <SetupKeyForm v-else-if="screen === 'key'" v-model="setupKey" :busy="submitting" :error="error" :key-error="keyError" @update:model-value="keyError = ''" @submit="submitKey" />
    <CreateOwnerForm v-else-if="screen === 'owner'" v-model="owner" :busy="submitting" :error="error" :field-errors="fieldErrors" @submit="submitOwner" />
  </AuthLayout>
</template>
