<template>
  <div>
    <div class="toolbar">
      <n-select v-model:value="kind" :options="kinds" style="width: 140px" />
      <n-select v-model:value="lines" :options="lineOpts" style="width: 110px" />
      <n-input v-model:value="unit" placeholder="服务单元，如 ssh.service" clearable style="width: 210px" />
      <n-select v-model:value="priority" :options="priorities" clearable placeholder="级别" style="width: 120px" />
      <n-input v-model:value="grep" placeholder="过滤关键词" clearable style="width: 170px" />
      <n-button @click="load" :loading="loading">刷新</n-button>
      <n-button :type="following ? 'error' : 'default'" @click="toggleFollow">
        {{ following ? '停止跟随' : '实时跟随' }}
      </n-button>
      <n-button @click="download">导出日志</n-button>
      <span class="muted">来源 {{ source }} · 共 {{ linesData.length }} 行</span>
    </div>
    <n-alert v-if="notice" type="warning" size="small" style="margin-bottom: 12px">{{ notice }}</n-alert>
    <div ref="box" class="code-block mono" style="max-height: calc(100vh - 200px); min-height: 320px">
      <div v-for="(l, i) in displayLines" :key="i" :style="{ color: colorOf(l) }">{{ l }}</div>
      <div v-if="!displayLines.length" class="muted">暂无日志</div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onUnmounted, nextTick } from 'vue'
import { useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const kinds = [
  { label: '系统日志', value: 'system' },
  { label: '内核日志', value: 'kernel' }
]
const lineOpts = [100, 200, 500, 1000, 2000].map((n) => ({ label: `${n} 行`, value: n }))
const priorities = ['emerg', 'alert', 'crit', 'err', 'warning', 'notice', 'info', 'debug'].map((v) => ({ label: v, value: v }))

const kind = ref('system')
const lines = ref(200)
const unit = ref('')
const priority = ref(null)
const grep = ref('')
const loading = ref(false)
const following = ref(false)
const source = ref('-')
const notice = ref('')
const linesData = ref([])
const box = ref(null)
let ws = null

const displayLines = computed(() => linesData.value.slice(-1200))

function query() {
  const q = { lines: lines.value }
  if (kind.value === 'kernel') q.kernel = true
  if (unit.value) q.unit = unit.value
  if (priority.value) q.priority = priority.value
  if (grep.value) q.grep = grep.value
  return q
}

function colorOf(l) {
  const s = l.toLowerCase()
  if (/(error|fail|crit|fatal|panic|denied)/.test(s)) return '#ff7b72'
  if (/(warn|notice|deprecat)/.test(s)) return '#e3b341'
  if (/(start|started|success|up|ready)/.test(s)) return '#7ee787'
  return ''
}

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/logs', { params: query() })
    source.value = data.source
    linesData.value = data.lines || []
    notice.value = data.available ? '' : '系统日志后端不可用，可能需要 root 权限或安装 rsyslog / systemd-journald。'
    await nextTick()
    scrollBottom()
  } catch (e) {
    message.error(String(errMsg(e)))
  } finally {
    loading.value = false
  }
}

function scrollBottom() {
  if (box.value) box.value.scrollTop = box.value.scrollHeight
}

function stopFollow() {
  following.value = false
  if (ws) { try { ws.close() } catch {} ws = null }
}

function toggleFollow() {
  if (following.value) { stopFollow(); return }
  linesData.value = []
  const q = query()
  const p = new URLSearchParams()
  p.set('token', localStorage.getItem('token') || '')
  if (q.kernel) p.set('kernel', 'true')
  if (q.unit) p.set('unit', q.unit)
  if (q.priority) p.set('priority', q.priority)
  if (q.grep) p.set('grep', q.grep)
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  ws = new WebSocket(`${proto}://${location.host}/api/logs/follow?${p.toString()}`)
  following.value = true
  source.value = '实时流'
  ws.onmessage = async (e) => {
    linesData.value.push(e.data)
    if (linesData.value.length > 3000) linesData.value.shift()
    await nextTick()
    scrollBottom()
  }
  ws.onerror = () => { message.error('实时日志连接失败'); stopFollow() }
  ws.onclose = () => { following.value = false; ws = null }
}

function download() {
  const blob = new Blob([linesData.value.join('\n')], { type: 'text/plain;charset=utf-8' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = `bianwrt-${kind.value}-${Date.now()}.log`
  a.click()
  URL.revokeObjectURL(a.href)
}

onUnmounted(stopFollow)
load()
</script>
