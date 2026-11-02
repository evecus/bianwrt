<template>
  <div style="height: calc(100vh - 40px)">
    <iframe
      v-if="src"
      :src="src"
      style="width: 100%; height: 100%; border: 0; border-radius: 6px; background: #fff"
    />
    <n-empty v-else description="插件不存在或未提供页面" style="padding-top: 120px" />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { NEmpty } from 'naive-ui'

const route = useRoute()
const src = computed(() =>
  route.params.name
    ? `/plugins/${encodeURIComponent(route.params.name)}/`
    : ''
)
</script>
