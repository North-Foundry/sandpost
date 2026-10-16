<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { LOCALE_NAMES, SUPPORTED_LOCALES, setLocale } from '@/i18n'

const { t, locale } = useI18n()
</script>

<template>
  <NativeSelect :model-value="locale" :aria-label="t('common.language')" size="sm" @update:model-value="setLocale">
    <NativeSelectOption v-for="language in SUPPORTED_LOCALES" :key="language" :value="language" :lang="language">
      {{ LOCALE_NAMES[language] }}
    </NativeSelectOption>
  </NativeSelect>
</template>
