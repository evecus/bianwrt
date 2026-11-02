<template>
  <n-card title="软件包管理 (apt)" style="margin-bottom: 16px">
    <n-space vertical>
      <n-space>
        <n-input v-model:value="q" placeholder="搜索已安装/仓库包名" style="width: 300px" clearable @keyup.enter="doSearch" />
        <n-button type="primary" @click="doSearch">搜索</n-button>
        <n-button @click="loadInstalled">刷新已安装</n-button>
        <n-tag :type="jobRunning ? 'warning' : 'default'" size="small">
          {{ jobRunning ? '任务执行中…' : '空闲' }}
        </n-tag>
      </n-space>
      <n-tabs type="line" v-model:value="tab">
        <n-tab-pane name="installed" tab="已安装">
          <n-data-table :columns="installedCols" :data="installed" size="small" :loading="loading" />
        </n-tab-pane>
        <n-tab-pane name="available" tab="仓库搜索结果">
          <n-data-table :columns="availCols" :data="available" size="small" />
        </n-tab-pane>
      </n-tabs>
    </n-space>
  </n-card>

  <n-card title="任务输出">
    <div ref="logBox" style="max-height: 320px; overflow-y: auto; font-family: monospace; font-size: 12px; line-height: 1.6; white-space: pre-wrap; background: #111; color: #9ae6b4; padding: 12px; border-radius: 6px">
      <div v-for="(l, i) in log" :key="i">{{ l }}</div>
    </div>
  </n-card>
</template>

<script setup>
import { ref, onMounted, onUnmounted, nextTick, h } from 'vue'
import { NCard, NSpace, NInput, NButton, NDataTable, NTabs, NTabPane, NTag, useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const q = ref('')
const tab = ref('installed')
const installed = ref([])
const available = ref([])
const loading = ref(false)
const log = ref([])
const jobRunning = ref(false)
const logBox = ref(null)
let ws = null

const actBtn = (label, type, op) => (r) =>
  h(NButton, {
    size: 'tiny', type, onClick: () => runAction(op, r.name)
  }, { default: () => label })

const installedCols = [
  { title: '包名', key: 'name', width: 240 },
  { title: '版本', key: 'version', width: 180 },
  { title: '说明', key: 'summary' },
  { title: '操作', key: 'op', width: 130, render: actBtn('卸载', 'error', 'remove') }
]
const availCols = [
  { title: '包名', key: 'name', width: 240 },
  { title: '说明', key: 'summary' },
  { title: '操作', key: 'op', width: 130, render: actBtn('安装', 'primary', 'install') }
]

async function loadInstalled() {
  loading.value = true
  try {
    const { data } = await http.get('/pkg/installed', { params: { q: q.value } })
    installed.value = data.items.filter(x => x.installed)
  } catch (e) { message.error(errMsg(e)) } finally { loading.value = false }
}
async function doSearch() {
  loading.value = true
  tab.value = 'available'
  try {
    const { data } = await http.get('/pkg/search', { params: { q: q.value } })
    available.value = data.items
    if (!available.value.length) message.info('仓库中未找到，试试已安装标签页')
  } catch (e) { message.error(errMsg(e)) } finally { loading.value = false }
}
async function runAction(op, name) {
  try {
    await http.post('/pkg/action', { op, packages: [name] })
    log.value.push(`>>> ${op} ${name}`)
    jobRunning.value = true
  } catch (e) {
    message.error(errMsg(e))
  }
}
function connectWs() {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const token = localStorage.getItem('token') || ''
  ws = new WebSocket(`${proto}://${location.host}/api/pkg/ws?token=${encodeURIComponent(token)}`)
  ws.onmessage = (ev) => {
    log.value.push(ev.data)
    if (log.value.length > 800) log.value.splice(0, log.value.length - 800)
    if (ev.data.startsWith('[bianwrt] 任务结束')) {
      jobRunning.value = false
      loadInstalled()
    }
    nextTick(() => { if (logBox.value) logBox.value.scrollTop = logBox.value.scrollHeight })
  }
  ws.onclose = () => { setTimeout(connectWs, 3000) }
}

onMounted(() => { loadInstalled(); connectWs() })
onUnmounted(() => { if (ws) ws.onclose = null; ws?.close() })
</script>
