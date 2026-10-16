<template><RouterView /></template>

<script setup lang="ts">
import { RouterView } from 'vue-router'
import { useTheme } from '@/composables/useTheme'

/*
 * Apply the persisted or system appearance before any page renders.
 */
useTheme()
</script>
