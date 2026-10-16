<script setup lang="ts">
import { ChevronDownIcon } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

defineProps<{ busy: boolean; error: string; keyError: string }>()
const setupKey = defineModel<string>({ required: true })
const { t } = useI18n()
const emit = defineEmits<{
  /**
   * Submit the entered setup key to the owning setup page.
   */
  submit: []
}>()
</script>

<template>
  <Card class="py-6">
    <form class="setup-key-form flex flex-col gap-6" :aria-busy="busy" @submit.prevent="emit('submit')">
      <CardHeader class="gap-2 px-6 text-center">
        <CardTitle><h1 class="text-xl font-semibold tracking-tight">{{ t('auth.setup.keyTitle') }}</h1></CardTitle>
        <CardDescription>{{ t('auth.setup.keyDescription') }}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-5 px-6">
        <Alert v-if="error" variant="destructive"><AlertDescription>{{ error }}</AlertDescription></Alert>
        <FieldGroup>
          <Field :data-invalid="Boolean(keyError)" :data-disabled="busy">
            <FieldLabel for="setup-key">{{ t('auth.setup.keyLabel') }}</FieldLabel>
            <Input
              id="setup-key"
              v-model="setupKey"
              class="h-9 font-mono"
              name="setupKey"
              autocomplete="off"
              autocapitalize="none"
              :spellcheck="false"
              :disabled="busy"
              :aria-invalid="Boolean(keyError)"
              :aria-describedby="keyError ? 'setup-key-error' : undefined"
              required
            />
            <FieldError v-if="keyError" id="setup-key-error">{{ keyError }}</FieldError>
          </Field>
        </FieldGroup>
      </CardContent>
      <CardFooter class="flex-col gap-4 px-6 py-5">
        <Button class="w-full" size="lg" type="submit" :disabled="busy">
          <Spinner v-if="busy" data-icon="inline-start" aria-hidden="true" />
          {{ busy ? t('auth.common.checking') : t('auth.common.continue') }}
        </Button>
        <Collapsible class="w-full">
          <CollapsibleTrigger as-child>
            <Button class="group w-full" variant="ghost" size="sm" type="button">
              {{ t('auth.setup.keyHelp') }}
              <ChevronDownIcon data-icon="inline-end" class="transition-transform group-data-[state=open]:rotate-180" aria-hidden="true" />
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <div class="flex flex-col gap-3 pt-3 text-sm leading-relaxed text-muted-foreground">
              <p>{{ t('auth.setup.keyHelpDescription') }}</p>
              <p><strong class="font-medium text-foreground">{{ t('auth.setup.docker') }}:</strong> {{ t('auth.setup.dockerHelp') }}</p>
              <p><strong class="font-medium text-foreground">{{ t('auth.setup.cli') }}:</strong> {{ t('auth.setup.cliHelp') }}</p>
            </div>
          </CollapsibleContent>
        </Collapsible>
      </CardFooter>
    </form>
  </Card>
</template>
