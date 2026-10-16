<script setup lang="ts">
import { onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import type { MessageSummary } from '@/api/contracts'
import { useSandpostClient } from '@/runtime/client'
import { SidebarInset, SidebarProvider } from '@/components/ui/sidebar'
import { adminContextKey, createAdminContext } from '@/features/admin/context'
import SearchBar from '@/features/mail/SearchBar.vue'
import AppSidebar from '@/features/shell/AppSidebar.vue'
import ShellHeader from '@/features/shell/ShellHeader.vue'
import { createWorkspace, DEFAULT_INBOX, workspaceKey } from '@/features/shell/workspace'

const client = useSandpostClient()
const route = useRoute()
const router = useRouter()

const { workspace, start, stop } = createWorkspace(client, {
  /**
   * Return to login when the workspace loses its authenticated session.
   */
  onSignedOut: () => router.replace({ name: 'login' })
})

provide(workspaceKey, workspace)
provide(adminContextKey, createAdminContext(workspace.user))

const search = ref('')

/**
 * Report whether the current route shows the message list.
 */
function onMailRoute(): boolean {
  return route.name === 'mail' || route.name === 'mail-message'
}

/**
 * The applied search, when the message list shows search results.
 */
function appliedSearch(): string {
  return onMailRoute() && typeof route.query.q === 'string' ? route.query.q : ''
}

/**
 * Show every match in the message list, keeping the folder to return to after clearing.
 */
function submitSearch(input: string): void {
  if (!input) { clearSearch(); return }

  const folder = onMailRoute() && typeof route.query.folder === 'string' ? { folder: route.query.folder } : {}
  void router.push({ name: 'mail', query: { ...folder, q: input } })
}

/**
 * Remove an applied search and return to the folder it was started from.
 */
function clearSearch(): void {
  if (!appliedSearch()) return

  const { q: _removed, ...query } = route.query
  void router.replace({ name: route.name ?? 'mail', params: route.params, query })
}

/**
 * Open a search result in the address inbox that received it, falling back to the inbox.
 */
function openResult(message: MessageSummary): void {
  const recipients = (message.envelopeTo || message.to).map((address) => address.toLowerCase())
  const folder = workspace.folders.value.find((candidate) => candidate.kind === 'address' && candidate.address && recipients.includes(candidate.address.toLowerCase()))

  void router.push({
    name: 'mail-message',
    params: { id: message.id },
    query: folder && folder.id !== DEFAULT_INBOX.id ? { folder: folder.id } : {},
  })
}

watch(() => route.fullPath, () => { search.value = appliedSearch() }, { immediate: true })

onMounted(start)
onBeforeUnmount(stop)
</script>

<template>
  <SidebarProvider class="h-dvh min-h-0 flex-col overflow-hidden">
    <ShellHeader>
      <SearchBar v-model:query="search" @submit="submitSearch" @select="openResult" @clear="clearSearch" />
    </ShellHeader>
    <div class="flex min-h-0 flex-1">
      <AppSidebar />
      <SidebarInset class="min-h-0 min-w-0 overflow-hidden">
        <RouterView />
      </SidebarInset>
    </div>
  </SidebarProvider>
</template>
