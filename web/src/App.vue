<template>
  <n-config-provider :theme="darkTheme" :locale="zhCN" :date-locale="dateZhCN" style="height: 100%">
    <n-message-provider><n-dialog-provider>
      <router-view />
    </n-dialog-provider></n-message-provider>
  </n-config-provider>
</template>

<script setup>
import { NConfigProvider, NMessageProvider, NDialogProvider, darkTheme, zhCN, dateZhCN } from 'naive-ui'
</script>
