<script setup lang="ts">
import { computed } from 'vue'
import { QUERY_TOKEN_CLASSES } from './tokenClasses'
import { highlightTokens } from './vocabulary'

const props = defineProps<{ source: string }>()

const tokens = computed(() => highlightTokens(props.source))
</script>

<template>
  <template v-for="(token, index) in tokens" :key="index"><span :class="QUERY_TOKEN_CLASSES[token.kind]">{{ token.text }}</span></template>
</template>
