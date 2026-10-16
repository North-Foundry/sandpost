<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'

defineProps<{ title: string; description: string; busy?: boolean; contentClass?: HTMLAttributes['class'] }>()
const open = defineModel<boolean>('open', { required: true })
</script>

<template>
  <Dialog :open="open" @update:open="!busy && (open = $event)">
    <DialogContent
      :class="cn('max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-md', contentClass)"
      :show-close-button="!busy"
      @escape-key-down="busy && $event.preventDefault()"
      @pointer-down-outside="busy && $event.preventDefault()"
    >
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription>{{ description }}</DialogDescription>
      </DialogHeader>
      <slot />
    </DialogContent>
  </Dialog>
</template>
