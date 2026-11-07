<template>
  <div>
    <div class="toolbar">
      <n-input v-model:value="kw" placeholder="搜索服务名 / 描述" clearable style="width: 260px" />
      <n-select v-model:value="filter" :options="filters" style="width: 150px" />
      <n-button @click="load" :loading="loading">刷新</n-button>
      <span class="muted">共 {{ items.length }} 个服务 · 显示 {{ filtered.length }} 个</span>
    </div>
    <n-data-table
      :columns="cols" :data="filtered" size="small" :bordered="false"
      :pagination="{ pageSize: 20 }" :row-key="(r) => r.unit" :max-height="'calc(100vh - 190px)'"
    />
    <n-drawer v-model:show="drawer" :width="640" placement="right">
      <n-drawer-content :title="current" closable>
        <pre class="code-block mono" style="max-height: calc(100vh - 160px)">{{ statusText || '无输出' }}</pre>
      </n-drawer-content>
    </n-drawer>
  </div>
</template>

<script setup>
import { ref, computed, h } from 'vue'
import { NTag, NButton, NSpace, NDropdown, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const items = ref([])
const kw = ref('')
const filter = ref('all')
const loading = ref(false)
const drawer = ref(false)
const current = ref('')
const statusText = ref('')

const filters = [
  { label: '全部服务', value: 'all' },
  { label: '运行中', value: 'active' },
  { label: '已启用开机启动', value: 'enabled' },
  { label: '未运行', value: 'inactive' }
]

const filtered = computed(() => {
  const k = kw.value.trim().toLowerCase()
  return items.value.filter((r) => {
    if (filter.value === 'active' && r.active !== 'active') return false
    if (filter.value === 'inactive' && r.active === 'active') return false
    if (filter.value === 'enabled' && r.enabled !== 'enabled') return false
    if (k && ![r.unit, r.description].join(' ').toLowerCase().includes(k)) return false
    return true
  })
})

function actions(r) {
  return [
    { label: '启动', key: 'start' },
    { label: '停止', key: 'stop' },
    { label: '重启', key: 'restart' },
    { type: 'divider', key: 'd1' },
    { label: '开机启用', key: 'enable' },
    { label: '禁用开机启动', key: 'disable' }
  ]
}

const cols = [
  { title: '服务单元', key: 'unit', width: 260, ellipsis: { tooltip: true } },
  {
    title: '运行状态', key: 'active', width: 120,
    render: (r) => h(NTag, { size: 'small', type: r.active === 'active' ? 'success' : r.active === 'failed' ? 'error' : 'default' }, { default: () => r.active })
  },
  { title: '子状态', key: 'sub', width: 120, render: (r) => r.sub || '-' },
  {
    title: '开机启动', key: 'enabled', width: 120,
    render: (r) => h(NTag, { size: 'small', type: r.enabled === 'enabled' ? 'info' : 'default' }, { default: () => r.enabled })
  },
  { title: '描述', key: 'description', ellipsis: { tooltip: true } },
  {
    title: '操作', key: 'act', width: 190,
    render: (r) => h(NSpace, { size: 'small' }, {
      default: () => [
        h(NDropdown, {
          trigger: 'click', options: actions(r),
          onSelect: (key) => doAction(r, key)
        }, { default: () => h(NButton, { size: 'tiny' }, { default: () => '操作 ▾' }) }),
        h(NButton, { size: 'tiny', quaternary: true, onClick: () => showStatus(r) }, { default: () => '详情' })
      ]
    })
  }
]

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/services')
    items.value = data.items || []
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function doAction(r, op) {
  const run = async () => {
    try {
      const { data } = await http.post('/services/action', { unit: r.unit, op })
      message.success(data.msg || '操作成功')
      setTimeout(load, 500)
    } catch (e) { message.error(String(errMsg(e))) }
  }
  if (op === 'stop' || op === 'restart') {
    dialog.warning({
      title: `确认 ${op}`,
      content: `确定要对 ${r.unit} 执行 ${op} 吗？服务会短暂中断。`,
      positiveText: '执行', negativeText: '取消', onPositiveClick: run
    })
  } else await run()
}

async function showStatus(r) {
  current.value = r.unit
  statusText.value = '加载中…'
  drawer.value = true
  try {
    const { data } = await http.get('/services/status', { params: { unit: r.unit } })
    statusText.value = data.text || '(无输出)'
  } catch (e) { statusText.value = String(errMsg(e)) }
}

load()
</script>
