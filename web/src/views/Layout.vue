<template>
  <n-layout style="height: 100vh" has-sider>
    <n-layout-sider bordered :width="210">
      <div style="display: flex; flex-direction: column; height: 100vh; box-sizing: border-box">
        <div style="padding: 18px 20px; font-size: 18px; font-weight: 700; color: #63e2b7">
          {{ menuConfig.brand || 'Bianwrt' }}
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
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { NLayout, NLayoutSider, NMenu, NButton } from 'naive-ui'
import { renderIcon, cloneMenu, fallbackMenu } from '../lib/menu'
import http from '../lib/http'

const route = useRoute()
const router = useRouter()
const menuConfig = ref(cloneMenu(fallbackMenu))
// LuCI 式「服务」分组：动态装载所有带页面（静态 / 反向代理）的插件
const pluginItems = ref([])

const menuOptions = computed(() => {
  const cfg = menuConfig.value
  const declared = new Set()
  cfg.groups.forEach((g) => g.items.forEach((i) => declared.add(i.key)))
  return cfg.groups
    .filter((g) => g.visible !== false)
    .map((g) => {
      const items = g.items
        .filter((i) => i.visible !== false)
        .map((i) => ({ label: i.label, key: i.key, icon: renderIcon(i.icon) }))
      // 未在菜单配置里声明的插件（如新装的）自动追加到「服务」分组
      if (g.key === 'grp-services') {
        for (const p of pluginItems.value) {
          const key = `/plugin-page/${p.name}`
          if (!declared.has(key)) {
            items.push({
              label: p.name,
              key,
              icon: renderIcon('ExtensionPuzzleOutline')
            })
          }
        }
      }
      return { type: 'group', label: g.label, key: g.key, children: items }
    })
    .filter((g) => g.children.length > 0)
})

const active = computed(() => route.path)

async function loadMenu() {
  try {
    const { data } = await http.get('/menu/config')
    menuConfig.value = data?.config?.groups?.length
      ? data.config
      : cloneMenu(fallbackMenu)
  } catch {
    menuConfig.value = cloneMenu(fallbackMenu)
  }
}
async function loadPlugins() {
  try {
    const { data } = await http.get('/plugins')
    pluginItems.value = (data.items || []).filter(
      (p) => p.web_mode !== 'none' && (p.has_ui || p.web_mode === 'proxy')
    )
  } catch { /* ignore */ }
}
function refresh() {
  loadMenu()
  loadPlugins()
}

watch(() => route.path, loadPlugins)
onMounted(() => {
  refresh()
  window.addEventListener('menu-config-changed', refresh)
})
onBeforeUnmount(() => window.removeEventListener('menu-config-changed', refresh))

function onMenu(key) {
  router.push(key)
}
async function logout() {
  try { await http.post('/auth/logout') } catch { /* ignore */ }
  localStorage.removeItem('token')
  router.push('/login')
}
</script>
