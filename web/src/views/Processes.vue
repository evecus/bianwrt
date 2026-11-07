<template>
  <div>
    <div class="toolbar">
      <n-input v-model:value="kw" placeholder="搜索 PID / 用户 / 命令" clearable style="width: 260px" />
      <n-button @click="load" :loading="loading">刷新</n-button>
      <n-switch v-model:value="auto" /> <span class="muted">每 5 秒自动刷新</span>
      <span class="muted">共 {{ items.length }} 个进程 · 显示 {{ filtered.length }} 个</span>
    </div>
    <n-data-table
      :columns="cols" :data="filtered" size="small" :bordered="false"
      :pagination="{ pageSize: 20 }" :row-key="(r) => r.pid" :max-height="'calc(100vh - 190px)'"
    />
  </div>
</template>

<script setup>
import { ref, computed, h, onMounted, onUnmounted } from 'vue'
import { NTag, NButton, NSpace, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const items = ref([])
const kw = ref('')
const loading = ref(false)
const auto = ref(false)
let timer = null

const filtered = computed(() => {
  const k = kw.value.trim().toLowerCase()
  if (!k) return items.value
  return items.value.filter((r) =>
    [r.pid, r.user, r.comm, r.args, r.stat].join(' ').toLowerCase().includes(k)
  )
})

const cols = [
  { title: 'PID', key: 'pid', width: 90, sorter: (a, b) => a.pid - b.pid },
  { title: '用户', key: 'user', width: 110 },
  {
    title: 'CPU%', key: 'cpu', width: 90, sorter: (a, b) => a.cpu - b.cpu,
    render: (r) => h('span', { style: { color: r.cpu > 50 ? '#ff7b72' : '' } }, r.cpu.toFixed(1))
  },
  {
    title: 'MEM%', key: 'mem', width: 90, sorter: (a, b) => a.mem - b.mem,
    render: (r) => r.mem.toFixed(1)
  },
  {
    title: '状态', key: 'stat', width: 90,
    render: (r) => h(NTag, { size: 'small', type: r.stat.includes('R') ? 'success' : 'default' }, { default: () => r.stat })
  },
  { title: '运行时长', key: 'etime', width: 110 },
  { title: '程序', key: 'comm', width: 130 },
  { title: '命令行', key: 'args', ellipsis: { tooltip: true } },
  {
    title: '操作', key: 'act', width: 150,
    render: (r) => h(NSpace, { size: 'small' }, {
      default: () => [
        h(NButton, { size: 'tiny', onClick: () => doKill(r, 'TERM') }, { default: () => 'TERM' }),
        h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => doKill(r, 'KILL') }, { default: () => 'KILL' })
      ]
    })
  }
]

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/processes')
    items.value = data.items || []
  } catch (e) {
    message.error(String(errMsg(e)))
  } finally {
    loading.value = false
  }
}

function doKill(r, signal) {
  dialog.warning({
    title: '确认发送信号',
    content: `向 PID ${r.pid} (${r.comm}) 发送 ${signal} 信号？进程将被${signal === 'KILL' ? '强制' : ''}终止。`,
    positiveText: '发送',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        const { data } = await http.post('/processes/kill', { pid: r.pid, signal })
        message.success(data.msg || '已发送')
        setTimeout(load, 400)
      } catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

onMounted(() => {
  load()
  timer = setInterval(() => { if (auto.value) load() }, 5000)
})
onUnmounted(() => clearInterval(timer))
</script>
