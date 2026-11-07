<template>
  <n-layout style="height: 100vh" has-sider>
    <n-layout-sider bordered :width="210">
      <div style="display: flex; flex-direction: column; height: 100vh; box-sizing: border-box">
        <div style="padding: 18px 20px; font-size: 18px; font-weight: 700; color: #63e2b7">
          Bianwrt
        </div>
        <div style="flex: 1; overflow-y: auto; overflow-x: hidden">
          <n-menu :value="active" :options="menuOptions" @update:value="onMenu" />
        </div>
        <div style="padding: 10px 16px; border-top: 1px solid rgba(255,255,255,0.09)">
          <n-button quaternary size="small" @click="logout">退出登录</n-button>
        </div>
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
  PricetagsOutline, RocketOutline, ExtensionPuzzleOutline,
  ReaderOutline, PeopleOutline, PulseOutline, ServerOutline,
  CalendarOutline, ArchiveOutline, SettingsOutline,
  LayersOutline, SpeedometerOutline, KeyOutline, FunnelOutline
} from '@vicons/ionicons5'
import http from '../lib/http'

const route = useRoute()
const router = useRouter()

const icon = (comp) => () => h(NIcon, null, { default: () => h(comp) })
// LuCI 式「服务」分组：动态装载所有带 web 页面的插件
const pluginItems = ref([])

// LuCI 式分组：状态 / 网络 / 系统 / 服务
const menuOptions = computed(() => [
  {
    type: 'group', label: '状态', key: 'grp-status', children: [
      { label: '仪表盘', key: '/dashboard', icon: icon(StatsChart) },
      { label: '系统日志', key: '/logs', icon: icon(ReaderOutline) },
      { label: '在线客户端', key: '/clients', icon: icon(PeopleOutline) },
      { label: '进程管理', key: '/processes', icon: icon(PulseOutline) }
    ]
  },
  {
    type: 'group', label: '网络', key: 'grp-network', children: [
      { label: '网络接口', key: '/network', icon: icon(GitNetwork) },
      { label: '路由与接口', key: '/routing', icon: icon(LayersOutline) },
      { label: '网络诊断', key: '/diagnostics', icon: icon(SpeedometerOutline) },
      { label: '防火墙', key: '/firewall', icon: icon(ShieldOutline) },
      { label: 'DHCP / DNS', key: '/dhcp', icon: icon(PlanetOutline) },
      { label: 'DNS 过滤', key: '/dnsfilter', icon: icon(FunnelOutline) },
      { label: 'WireGuard', key: '/wireguard', icon: icon(KeyOutline) }
    ]
  },
  {
    type: 'group', label: '系统', key: 'grp-system', children: [
      { label: '系统设置', key: '/system', icon: icon(SettingsOutline) },
      { label: '服务与启动项', key: '/services', icon: icon(ServerOutline) },
      { label: '计划任务', key: '/cron', icon: icon(CalendarOutline) },
      { label: '备份 / 恢复', key: '/backup', icon: icon(ArchiveOutline) },
      { label: '软件包', key: '/packages', icon: icon(PricetagsOutline) }
    ]
  },
  {
    type: 'group', label: '服务', key: 'grp-services', children: [
      { label: '插件中心', key: '/plugins', icon: icon(RocketOutline) },
      ...pluginItems.value
    ]
  }
])

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
  try { await http.post('/auth/logout') } catch { /* ignore */ }
  localStorage.removeItem('token')
  router.push('/login')
}
</script>
