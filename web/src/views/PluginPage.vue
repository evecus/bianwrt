<template>
  <div style="height: calc(100vh - 40px)">
    <iframe
      v-if="src"
      ref="frame"
      :src="src"
      title="plugin-page"
      sandbox="allow-scripts allow-forms allow-popups allow-downloads"
      referrerpolicy="no-referrer"
      style="width: 100%; height: 100%; border: 0; border-radius: 6px; background: #fff"
      @load="onFrameLoad"
    />
    <n-empty v-else description="插件不存在或未提供页面" style="padding-top: 120px" />
  </div>
</template>

<script setup>
import { computed, ref, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import { NEmpty, useMessage } from 'naive-ui'
import http from '../lib/http'

const route = useRoute()
const message = useMessage()
const frame = ref(null)
const ticket = ref('')
const name = computed(() => (route.params.name ? String(route.params.name) : ''))
// iframe 用受限票据 URL（/plugins/<name>/_bw/<ticket>/）：
// opaque origin 下相对资源与 fetch 都会自动带上票据前缀，无需 cookie。
const src = computed(() => {
  if (!name.value || !ticket.value) return ''
  return `/plugins/${encodeURIComponent(name.value)}/_bw/${ticket.value}/`
})

async function loadTicket() {
  ticket.value = ''
  if (!name.value) return
  try {
    const { data } = await http.post(`/plugin-ticket/${encodeURIComponent(name.value)}`)
    ticket.value = data.ticket || ''
  } catch { ticket.value = '' }
}

// iframe 不带 allow-same-origin：插件页是 opaque origin，读不到管理员 token。
// 官方 SDK（sdk/bianwrt-sdk.js）通过本桥与面板通信：
//   - HTTP 方法 → 父页面用已鉴权会话代发 /api/
//   - STORAGE_* → 父页面代管插件专属命名空间（插件页无法访问 localStorage）
//   - NOTIFY    → 借用面板的全局提示
//   - PING      → 插件名 / 主题
function post(msg) {
  try { frame.value?.contentWindow?.postMessage(msg, '*') } catch { /* ignore */ }
}
function reply(id, ok, data) {
  post({
    __bianwrt: 1, id, ok, data,
    error: ok ? undefined : (typeof data === 'string' ? data : JSON.stringify(data))
  })
}

function onFrameLoad() {
  post({ __bianwrt: 1, type: 'hello', plugin: name.value, theme: 'dark', sdk: true })
}

const kvKey = computed(() => `bw-plugin-kv:${name.value}`)
function readKv() {
  try { return JSON.parse(localStorage.getItem(kvKey.value) || '{}') || {} } catch { return {} }
}
function writeKv(o) {
  localStorage.setItem(kvKey.value, JSON.stringify(o))
}
function safeKey(k) {
  return typeof k === 'string' && /^[A-Za-z0-9_.:-]{1,64}$/.test(k)
}
function safeValue(v) {
  try { return JSON.stringify(v ?? null).length <= 65536 } catch { return false }
}

async function onMessage(ev) {
  if (!frame.value || ev.source !== frame.value.contentWindow) return
  const m = ev.data
  if (!m || m.__bianwrt !== 1 || typeof m.id === 'undefined') return
  const method = String(m.method || 'GET').toUpperCase()

  // ── 内部桥：插件专属 KV 存储（opaque origin 下 localStorage 不可用） ──
  if (method.startsWith('STORAGE_')) {
    const key = m.key
    if (method !== 'STORAGE_KEYS' && !safeKey(key)) {
      return reply(m.id, false, '非法存储键（仅允许 A-Z a-z 0-9 _ . : -，≤64 字符）')
    }
    try {
      const kv = readKv()
      if (method === 'STORAGE_GET') return reply(m.id, true, Object.prototype.hasOwnProperty.call(kv, key) ? kv[key] : null)
      if (method === 'STORAGE_KEYS') return reply(m.id, true, Object.keys(kv))
      if (method === 'STORAGE_DEL') {
        delete kv[key]
        writeKv(kv)
        return reply(m.id, true, true)
      }
      if (method === 'STORAGE_SET') {
        if (!safeValue(m.value)) return reply(m.id, false, '存储值过大或无法序列化（上限 64KB）')
        kv[key] = m.value
        writeKv(kv)
        return reply(m.id, true, true)
      }
      if (method === 'STORAGE_CLEAR') {
        writeKv({})
        return reply(m.id, true, true)
      }
      return reply(m.id, false, '未知的存储操作')
    } catch (e) {
      return reply(m.id, false, String(e?.message || e))
    }
  }
  if (method === 'NOTIFY') {
    const type = ['success', 'info', 'warning', 'error'].includes(m.type) ? m.type : 'info'
    const text = String(m.text || '').slice(0, 300)
    if (text) message[type](text)
    return reply(m.id, true, true)
  }
  if (method === 'PING') {
    return reply(m.id, true, { plugin: name.value, theme: 'dark', sdk: true })
  }

  // ── API 代理：只允许 /api/ 前缀，父页面用已登录会话代发 ──
  const path = String(m.path || '')
  if (!path.startsWith('/api/')) return reply(m.id, false, '仅允许访问 /api/ 路径')
  if (!['GET', 'POST', 'PUT', 'DELETE', 'PATCH'].includes(method)) {
    return reply(m.id, false, '不支持的请求方法')
  }
  try {
    const r = await http.request({
      url: path.slice(4), // baseURL 已是 /api
      method,
      data: m.data,
      params: m.params
    })
    reply(m.id, true, r.data)
  } catch (e) {
    reply(m.id, false, e?.response?.data || e?.message || '请求失败')
  }
}

onMounted(() => {
  loadTicket()
  window.addEventListener('message', onMessage)
})
watch(name, loadTicket)
onBeforeUnmount(() => window.removeEventListener('message', onMessage))
</script>
