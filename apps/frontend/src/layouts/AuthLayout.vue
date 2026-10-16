<script setup lang="ts">
import { MoonIcon, SunIcon } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import BrandMark from '@/components/BrandMark.vue'
import LanguageSelector from '@/components/LanguageSelector.vue'
import { useTheme } from '@/composables/useTheme'

defineProps<{ label: string }>()
const { isDark, toggleTheme } = useTheme()
const { t } = useI18n()
</script>

<template>
  <main class="relative flex min-h-dvh items-center justify-center bg-muted/40 px-4 py-16 sm:px-6">
    <div class="window-drag absolute inset-x-0 top-0 h-12" aria-hidden="true" />
    <div class="absolute top-4 right-4 flex items-center gap-2 sm:top-6 sm:right-6">
      <LanguageSelector />
      <Button
        variant="ghost"
        size="icon"
        type="button"
        :aria-label="isDark ? t('auth.theme.light') : t('auth.theme.dark')"
        @click="toggleTheme()"
      >
        <SunIcon v-if="isDark" aria-hidden="true" />
        <MoonIcon v-else aria-hidden="true" />
      </Button>
    </div>
    <section class="flex w-full max-w-sm flex-col items-center gap-6" :aria-label="label">
      <BrandMark />
      <div class="w-full"><slot /></div>
      <p class="text-center text-xs text-muted-foreground">{{ t('auth.brand.tagline') }}</p>
    </section>
  </main>
</template>
