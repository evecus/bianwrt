<template>
  <n-layout style="height: 100vh" has-sider>
    <n-layout-sider bordered :width="210">
      <div style="padding: 18px 20px; font-size: 18px; font-weight: 700; color: #63e2b7">
        Bianwrt
      </div>
      <n-menu :value="active" :options="menuOptions" @update:value="onMenu" />
      <div style="position: absolute; bottom: 12px; left: 20px">
        <n-button quaternary size="small" @click="logout">退出登录</n-button>
      </div>
    </n-layout-sider>
    <n-layout content-style="padding: 20px;">
      <router-view />
    </n-layout>
  </n-layout>
</template>

<script setup>
import { ref, computed, watch, h } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { NLayout, NLayoutSider, NMenu, NButton, NIcon } from 'naive-ui'
import {
  StatsChart, GitNetwork, ShieldOutline, PlanetOutline,
  PricetagsOutline, RocketOutline, ExtensionPuzzleOutline
} from '@vicons/ionicons5'
import http from '../lib/http'

const route = useRoute()
const router = useRouter()

const icon = (comp) => () => h(NIcon, null, { default: () => h(comp) })
// LuCI 式「服务」分组：动态装载所有带 web 页面的插件
const pluginItems = ref([])

const menuOptions = computed(() => {
  const base = [
    { label: '仪表盘', key: '/dashboard', icon: icon(StatsChart) },
    { label: '网络接口', key: '/network', icon: icon(GitNetwork) },
    { label: '防火墙', key: '/firewall', icon: icon(ShieldOutline) },
    { label: 'DHCP / DNS', key: '/dhcp', icon: icon(PlanetOutline) },
    { label: '软件包', key: '/packages', icon: icon(PricetagsOutline) },
    { label: '插件中心', key: '/plugins', icon: icon(RocketOutline) }
  ]
  if (!pluginItems.value.length) return base
  return [
    ...base,
    { type: 'group', label: '服务', key: 'grp-services', children: pluginItems.value }
  ]
})

const active = computed(() => route.path)
async function loadPlugins() {
  try {
    const { data } = await http.get('/plugins')
    pluginItems.value = data.items
      .filter((p) => p.web_mode === 'static')
      .map((p) => ({
        label: p.name,
        key: `/plugin-page/${p.name}`,
        icon: icon(ExtensionPuzzleOutline)
      }))
  } catch { /* ignore */ }
}
watch(() => route.path, loadPlugins, { immediate: true })

function onMenu(key) {
  router.push(key)
}
async function logout() {
  localStorage.removeItem('token')
  router.push('/login')
}
</script>
