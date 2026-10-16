<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { onMounted, ref } from 'vue'
import type { Scope, UserAccount } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { describeError } from '@/utils/errors'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import ScopesPanel from '@/features/admin/ScopesPanel.vue'

const client = useSandpostClient()
const scopes = ref<Scope[]>([])
const { t } = useI18n()
const users = ref<UserAccount[]>([])

/**
 * Assigned user identifiers keyed by scope identifier.
 */
const assignments = ref<Record<string, string[]>>({})
const loaded = ref(false)
const error = ref('')

/**
 * Load every scope, the assignable accounts, and each scope's assignments.
 */
async function load(): Promise<void> {
  error.value = ''
  try {
    const [loadedScopes, loadedUsers] = await Promise.all([client.admin.scopes.list(), client.admin.users.list()])
    const loadedAssignments = await Promise.all(loadedScopes.map((scope) => client.admin.scopes.assignments(scope.id)))
    scopes.value = loadedScopes
    users.value = loadedUsers.sort((first, second) => first.name.localeCompare(second.name))
    assignments.value = Object.fromEntries(loadedScopes.map((scope, index) => [scope.id, loadedAssignments[index]!]))
    loaded.value = true
  } catch (failure) {
    error.value = describeError(failure, t('admin.scopes.loadError'))
  }
}

onMounted(load)
</script>

<template>
  <div class="mx-auto flex max-w-6xl flex-col gap-6 p-4 sm:p-6">
    <div class="flex flex-col gap-1">
      <h1 class="text-lg font-semibold tracking-tight">{{ t('admin.scopes.title') }}</h1>
      <p class="text-sm text-muted-foreground">{{ t('admin.scopes.description') }}</p>
    </div>
    <Alert v-if="error" variant="destructive" role="alert">
      <AlertDescription class="flex items-center gap-3"><span class="flex-1">{{ error }}</span><Button variant="outline" size="sm" @click="load">{{ t('admin.common.retry') }}</Button></AlertDescription>
    </Alert>
    <div v-if="!loaded && !error" class="grid gap-4 lg:grid-cols-[17rem_minmax(0,1fr)]">
      <Skeleton class="h-64 w-full" />
      <Skeleton class="h-96 w-full" />
    </div>
    <ScopesPanel v-else-if="loaded" :scopes="scopes" :users="users" :assignments="assignments" @changed="load" />
  </div>
</template>
