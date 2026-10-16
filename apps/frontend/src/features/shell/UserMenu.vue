<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, ref } from 'vue'
import { ChevronsUpDownIcon, LanguagesIcon, LogOutIcon, MoonIcon, SunIcon } from '@lucide/vue'
import { describeError } from '@/utils/errors'
import { cn } from '@/lib/utils'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
  DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu'
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem } from '@/components/ui/sidebar'
import { Spinner } from '@/components/ui/spinner'
import { LOCALE_NAMES, SUPPORTED_LOCALES, setLocale } from '@/i18n'
import { useTheme } from '@/composables/useTheme'
import { useWorkspace } from './workspace'

const { t, locale } = useI18n()

const workspace = useWorkspace()
const { isDark, toggleTheme } = useTheme()
const loggingOut = ref(false)
const logoutError = ref('')

/**
 * A live connection is the normal state, so only connecting and offline are shown.
 */
const status = computed(() => ({
  connected: null,
  connecting: { label: t('common.connecting'), detail: t('common.waitingServer'), dot: 'bg-warning animate-pulse' },
  disconnected: { label: t('common.offline'), detail: t('common.reconnecting'), dot: 'bg-destructive' },
})[workspace.connection.value])
const initial = computed(() => (workspace.user.value?.name.trim().charAt(0) || '?').toUpperCase())

/**
 * Revoke the session; the workspace returns to the login page.
 */
async function logout(): Promise<void> {
  if (loggingOut.value) return
  loggingOut.value = true
  logoutError.value = ''
  try {
    await workspace.logout()
  } catch (error) {
    logoutError.value = describeError(error, t('common.logoutFailed'))
  } finally {
    loggingOut.value = false
  }
}
</script>

<template>
  <div v-if="status" class="flex items-center gap-2 px-2 pb-1 text-xs text-sidebar-foreground/70 group-data-[collapsible=icon]:hidden" role="status" :title="status.detail">
    <span :class="cn('size-1.5 shrink-0 rounded-full', status.dot)" aria-hidden="true" />
    <span class="shrink-0">{{ status.label }}</span>
    <span class="min-w-0 truncate text-sidebar-foreground/50">· {{ status.detail }}</span>
  </div>
  <p v-if="logoutError" class="px-2 text-xs text-destructive" role="alert">{{ logoutError }}</p>
  <SidebarMenu>
    <SidebarMenuItem>
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <SidebarMenuButton size="lg" class="data-[state=open]:bg-sidebar-accent group-data-[collapsible=icon]:size-10!" :aria-label="t('common.accountAppearance')">
            <span class="relative flex size-8 shrink-0 items-center group-data-[collapsible=icon]:size-10 justify-center rounded-md bg-sidebar-accent text-xs font-semibold" aria-hidden="true">
              {{ initial }}
              <!--
                The icon rail hides the status line, so the avatar carries its dot.
              -->
              <span v-if="status" :class="cn('absolute -top-0.5 -right-0.5 hidden size-2 rounded-full ring-2 ring-sidebar group-data-[collapsible=icon]:block', status.dot)" />
            </span>
            <span class="grid min-w-0 flex-1 leading-tight">
              <span class="truncate font-medium">{{ workspace.user.value?.name ?? '…' }}</span>
              <span class="truncate text-xs text-sidebar-foreground/60">{{ workspace.user.value?.email }}</span>
            </span>
            <ChevronsUpDownIcon class="ml-auto text-sidebar-foreground/50" aria-hidden="true" />
          </SidebarMenuButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent side="top" align="start" class="w-(--reka-dropdown-menu-trigger-width) min-w-56">
          <DropdownMenuLabel class="flex flex-col font-normal">
            <span class="truncate text-sm font-medium">{{ workspace.user.value?.name }}</span>
            <span class="truncate text-xs text-muted-foreground">{{ workspace.user.value?.email }} · <span class="capitalize">{{ workspace.user.value ? t(`common.roles.${workspace.user.value.role}`) : '' }}</span></span>
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger><LanguagesIcon />{{ t('common.language') }}</DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuRadioGroup :model-value="locale" @update:model-value="setLocale">
                  <DropdownMenuRadioItem v-for="language in SUPPORTED_LOCALES" :key="language" :value="language" :lang="language">
                    {{ LOCALE_NAMES[language] }}
                  </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuItem @select="toggleTheme()">
              <SunIcon v-if="isDark" /><MoonIcon v-else />{{ isDark ? t('common.lightAppearance') : t('common.darkAppearance') }}
            </DropdownMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuItem :disabled="loggingOut" :aria-label="t('common.signOut')" @select="logout">
              <Spinner v-if="loggingOut" aria-hidden="true" /><LogOutIcon v-else />{{ loggingOut ? t('common.signingOut') : t('common.signOut') }}
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </SidebarMenuItem>
  </SidebarMenu>
</template>
