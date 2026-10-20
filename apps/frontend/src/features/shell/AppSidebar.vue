<script setup lang="ts">
import { Sidebar, SidebarContent, SidebarFooter, SidebarRail } from '@/components/ui/sidebar'
import AdministrationNav from './AdministrationNav.vue'
import MailNavigation from './MailNavigation.vue'
import PersonaNavigation from './PersonaNavigation.vue'
import UserMenu from './UserMenu.vue'
</script>

<template>
  <!--
    Sits below the shell header, which holds the brand and toggle; collapsing leaves an icon rail.
  -->
  <Sidebar collapsible="icon" class="top-12 h-[calc(100svh-3rem)]">
    <SidebarContent class="gap-1 py-1">
      <MailNavigation />
      <PersonaNavigation />
      <AdministrationNav />
    </SidebarContent>
    <SidebarFooter class="gap-1 border-t border-sidebar-border">
      <UserMenu />
    </SidebarFooter>
    <SidebarRail />
  </Sidebar>
</template>
