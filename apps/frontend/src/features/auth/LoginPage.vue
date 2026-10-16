<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { useSandpostClient } from '@/runtime/client'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import AuthLayout from '@/layouts/AuthLayout.vue'

const client = useSandpostClient()
const router = useRouter()
const { t } = useI18n()
const email = ref('')
const password = ref('')
const error = ref('')
const submitting = ref(false)

/**
 * Authenticates the owner and opens the mail interface.
 */
async function submitLogin(): Promise<void> {
  error.value = ''
  submitting.value = true
  try {
    await client.login({ email: email.value.trim(), password: password.value })
    await router.replace({ name: 'mail' })
  } catch {
    error.value = t('auth.login.failed')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AuthLayout :label="t('auth.login.ariaLabel')">
    <Card class="py-6">
      <form class="flex flex-col gap-6" :aria-busy="submitting" @submit.prevent="submitLogin">
        <CardHeader class="gap-2 px-6 text-center">
          <CardTitle><h1 class="text-xl font-semibold tracking-tight">{{ t('auth.login.title') }}</h1></CardTitle>
          <CardDescription>{{ t('auth.login.description') }}</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-col gap-5 px-6">
          <Alert v-if="error" variant="destructive" role="alert"><AlertDescription>{{ error }}</AlertDescription></Alert>
          <FieldGroup>
            <Field :data-disabled="submitting">
              <FieldLabel for="login-email">{{ t('auth.common.email') }}</FieldLabel>
              <Input id="login-email" v-model="email" class="h-9" name="email" type="email" autocomplete="username" :disabled="submitting" required />
            </Field>
            <Field :data-disabled="submitting">
              <FieldLabel for="login-password">{{ t('auth.common.password') }}</FieldLabel>
              <Input id="login-password" v-model="password" class="h-9" name="password" type="password" autocomplete="current-password" minlength="12" maxlength="1024" aria-describedby="login-password-policy" :disabled="submitting" required />
              <FieldDescription id="login-password-policy">{{ t('auth.common.passwordPolicy') }}</FieldDescription>
            </Field>
          </FieldGroup>
        </CardContent>
        <CardFooter class="px-6 py-5">
          <Button class="w-full" size="lg" type="submit" :disabled="submitting">
            <Spinner v-if="submitting" data-icon="inline-start" aria-hidden="true" />
            {{ submitting ? t('auth.login.submitting') : t('auth.login.submit') }}
          </Button>
        </CardFooter>
      </form>
    </Card>
  </AuthLayout>
</template>
