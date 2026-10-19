<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { KeyRoundIcon, LayersIcon, MailboxIcon, UsersIcon } from '@lucide/vue'
import { SidebarGroup, SidebarGroupContent, SidebarGroupLabel, SidebarMenu } from '@/components/ui/sidebar'
import SidebarNavItem from './SidebarNavItem.vue'
import { useWorkspace } from './workspace'

const { t } = useI18n()

const route = useRoute()
const workspace = useWorkspace()

/*
 * Every signed-in user manages their own IMAP accounts; the other sections are for owners,
 * and the API refuses everyone else as well.
 */
const sections = computed(() => [
  ...(workspace.canAdminister.value
    ? [
      { name: 'admin-users', label: t('common.users'), icon: UsersIcon, routes: ['admin-users'] },
      { name: 'admin-scopes', label: t('common.scopes'), icon: LayersIcon, routes: ['admin-scopes'] },
      { name: 'admin-smtp-access', label: 'SMTP', icon: KeyRoundIcon, routes: ['admin-smtp-access'] },
    ]
    : []),
  { name: 'imap', label: 'IMAP', icon: MailboxIcon, routes: ['imap', 'imap-account'] },
])

/**
 * Report whether a section owns the current route.
 */
function isCurrent(routes: string[]): boolean {
  return typeof route.name === 'string' && routes.includes(route.name)
}
</script>

<template>
  <SidebarGroup class="mt-auto">
    <SidebarGroupLabel>{{ t('common.administration') }}</SidebarGroupLabel>
    <SidebarGroupContent>
      <SidebarMenu>
        <SidebarNavItem
          v-for="section in sections"
          :key="section.name"
          :to="{ name: section.name }"
          :icon="section.icon"
          :label="section.label"
          :active="isCurrent(section.routes)"
        />
      </SidebarMenu>
    </SidebarGroupContent>
  </SidebarGroup>
</template>
