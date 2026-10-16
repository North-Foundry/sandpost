<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { RouterLink, RouterView } from 'vue-router'
import { InboxIcon, ShieldOffIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { useWorkspace } from '@/features/shell/workspace'

const { t } = useI18n()

/*
 * Administration renders inside the application shell; the shell resolves the session and capability once.
 */
const { canAdminister, capabilitiesLoaded } = useWorkspace()
</script>

<template>
  <section class="h-full overflow-y-auto" :aria-label="t('common.administration')">
    <div v-if="!capabilitiesLoaded" class="mx-auto flex max-w-6xl flex-col gap-4 p-4 sm:p-6">
      <Skeleton class="h-7 w-48" />
      <Skeleton class="h-16 w-full" />
      <Skeleton class="h-16 w-full" />
    </div>
    <Empty v-else-if="!canAdminister" class="h-full">
      <EmptyHeader>
        <EmptyMedia variant="icon"><ShieldOffIcon /></EmptyMedia>
        <EmptyTitle>{{ t('common.noAdminAccess') }}</EmptyTitle>
        <EmptyDescription>{{ t('common.adminAccessHint') }}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent><Button as-child variant="outline"><RouterLink :to="{ name: 'mail' }"><InboxIcon data-icon="inline-start" />{{ t('common.openInbox') }}</RouterLink></Button></EmptyContent>
    </Empty>
    <RouterView v-else />
  </section>
</template>
