<script setup lang="ts">
import type { SetupInput } from '@/api/contracts'
import { useI18n } from 'vue-i18n'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

defineProps<{
  busy: boolean
  error: string
  fieldErrors: Partial<Record<'name' | 'email' | 'password' | 'confirmation', string>>
}>()
const owner = defineModel<Omit<SetupInput, 'bootstrapSecret'> & { confirmation: string }>({ required: true })
const { t } = useI18n()
const emit = defineEmits<{
  /**
   * Submit the owner details to the owning setup page.
   */
  submit: []
}>()
</script>

<template>
  <Card class="py-6">
    <form class="setup-owner-form flex flex-col gap-6" :aria-busy="busy" @submit.prevent="emit('submit')">
      <CardHeader class="gap-2 px-6 text-center">
        <CardTitle><h1 class="text-xl font-semibold tracking-tight">{{ t('auth.setup.ownerTitle') }}</h1></CardTitle>
        <CardDescription>{{ t('auth.setup.ownerDescription') }}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-5 px-6">
        <Alert v-if="error" variant="destructive"><AlertDescription>{{ error }}</AlertDescription></Alert>
        <FieldGroup>
          <Field :data-invalid="Boolean(fieldErrors.name)" :data-disabled="busy">
            <FieldLabel for="owner-name">{{ t('auth.common.name') }}</FieldLabel>
            <Input id="owner-name" v-model="owner.name" class="h-9" name="name" autocomplete="name" maxlength="100" :disabled="busy" :aria-invalid="Boolean(fieldErrors.name)" :aria-describedby="fieldErrors.name ? 'owner-name-error' : undefined" required />
            <FieldError v-if="fieldErrors.name" id="owner-name-error">{{ fieldErrors.name }}</FieldError>
          </Field>
          <Field :data-invalid="Boolean(fieldErrors.email)" :data-disabled="busy">
            <FieldLabel for="owner-email">{{ t('auth.common.email') }}</FieldLabel>
            <Input id="owner-email" v-model="owner.email" class="h-9" name="email" type="email" autocomplete="email" maxlength="254" :disabled="busy" :aria-invalid="Boolean(fieldErrors.email)" :aria-describedby="fieldErrors.email ? 'owner-email-error' : undefined" required />
            <FieldError v-if="fieldErrors.email" id="owner-email-error">{{ fieldErrors.email }}</FieldError>
          </Field>
          <Field :data-invalid="Boolean(fieldErrors.password)" :data-disabled="busy">
            <FieldLabel for="owner-password">{{ t('auth.common.password') }}</FieldLabel>
            <Input id="owner-password" v-model="owner.password" class="h-9" name="password" type="password" autocomplete="new-password" minlength="12" maxlength="1024" :disabled="busy" :aria-invalid="Boolean(fieldErrors.password)" :aria-describedby="fieldErrors.password ? 'password-policy owner-password-error' : 'password-policy'" required />
            <FieldDescription id="password-policy">{{ t('auth.common.passwordPolicy') }}</FieldDescription>
            <FieldError v-if="fieldErrors.password" id="owner-password-error">{{ fieldErrors.password }}</FieldError>
          </Field>
          <Field :data-invalid="Boolean(fieldErrors.confirmation)" :data-disabled="busy">
            <FieldLabel for="owner-confirmation">{{ t('auth.setup.confirmPassword') }}</FieldLabel>
            <Input id="owner-confirmation" v-model="owner.confirmation" class="h-9" name="confirmation" type="password" autocomplete="new-password" minlength="12" maxlength="1024" :disabled="busy" :aria-invalid="Boolean(fieldErrors.confirmation)" :aria-describedby="fieldErrors.confirmation ? 'owner-confirmation-error' : undefined" required />
            <FieldError v-if="fieldErrors.confirmation" id="owner-confirmation-error">{{ fieldErrors.confirmation }}</FieldError>
          </Field>
        </FieldGroup>
      </CardContent>
      <CardFooter class="px-6 py-5">
        <Button class="w-full" size="lg" type="submit" :disabled="busy">
          <Spinner v-if="busy" data-icon="inline-start" aria-hidden="true" />
          {{ busy ? t('auth.setup.creatingAccount') : t('auth.setup.createAccount') }}
        </Button>
      </CardFooter>
    </form>
  </Card>
</template>
