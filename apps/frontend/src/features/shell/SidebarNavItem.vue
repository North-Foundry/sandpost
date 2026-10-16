<script setup lang="ts">
import { useSlots, type Component } from 'vue'
import { RouterLink, type RouteLocationRaw } from 'vue-router'
import { cn } from '@/lib/utils'
import { SidebarMenuBadge, SidebarMenuButton, SidebarMenuItem, useSidebar } from '@/components/ui/sidebar'

const props = defineProps<{
  to: RouteLocationRaw
  icon: Component
  label: string
  active?: boolean

  /**
   * Secondary text shown in the native tooltip, such as an inbox address.
   */
  hint?: string

  /**
   * Short count shown at the trailing edge, such as unread mail.
   */
  badge?: string

  /**
   * Spoken description of the badge, such as "12 unread".
   */
  badgeLabel?: string
}>()
const slots = useSlots()
const { isMobile, setOpenMobile } = useSidebar()

/**
 * Close the mobile drawer once a destination is chosen.
 */
function navigated(): void {
  if (isMobile.value) setOpenMobile(false)
}
</script>

<template>
  <SidebarMenuItem>
    <SidebarMenuButton
      as-child
      :tooltip="props.label"
      :is-active="props.active"
      :class="cn('h-9 font-normal [&_svg]:size-5 group-data-[collapsible=icon]:size-10! group-data-[collapsible=icon]:p-2.5! hover:bg-sidebar-accent/60 [&>svg]:text-sidebar-foreground/55 data-active:font-medium data-active:hover:bg-sidebar-accent data-active:[&>svg]:text-sidebar-foreground')"
    >
      <RouterLink :to="props.to" :aria-current="props.active ? 'page' : undefined" :title="props.hint" @click="navigated">
        <component :is="props.icon" aria-hidden="true" />
        <!--
          The rail's larger button leaves room past the icon, so the label stays readable only to assistive tech.
        -->
        <span class="group-data-[collapsible=icon]:sr-only">{{ props.label }}</span>
        <span v-if="props.badge && props.badgeLabel" class="sr-only">, {{ props.badgeLabel }}</span>
      </RouterLink>
    </SidebarMenuButton>
    <SidebarMenuBadge
      v-if="props.badge"
      aria-hidden="true"
      :class="cn(
        'rounded-sm bg-primary text-primary-foreground peer-data-[size=default]/menu-button:top-2 peer-hover/menu-button:text-primary-foreground peer-data-active/menu-button:text-primary-foreground',
        /*
         * The badge makes room for the item's action: it slides left on hover, focus, or while the menu is open, and
         * sits beside the always-visible action on narrow screens.
         */
        slots.action && 'transition-transform duration-200 ease-out max-md:right-7 md:group-focus-within/menu-item:-translate-x-6 md:group-hover/menu-item:-translate-x-6 md:group-has-aria-expanded/menu-item:-translate-x-6',
      )"
    >
      {{ props.badge }}
    </SidebarMenuBadge>
    <slot name="action" />
  </SidebarMenuItem>
</template>
