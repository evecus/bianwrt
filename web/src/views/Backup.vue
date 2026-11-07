<template>
  <div>
    <n-alert type="info" size="small" style="margin-bottom: 12px" title="备份内容">
      包含面板数据库（防火墙 / DHCP / DNS 过滤 / 插件元数据）、conf 目录、生成的 nftables 与 dnsmasq 配置、
      crontab、WireGuard 与 ifupdown 配置。恢复时保留当前管理员账号与登录会话，避免把用户锁在门外。
    </n-alert>
    <div class="toolbar">
      <n-button type="primary" @click="create" :loading="creating">创建备份</n-button>
      <n-button @click="fileEl.click()">上传备份</n-button>
      <input ref="fileEl" type="file" accept=".tar.gz,.gz,.tgz" style="display: none" @change="onFile" />
      <n-button @click="load" :loading="loading">刷新列表</n-button>
      <span class="muted">{{ dir }}</span>
    </div>
    <n-data-table :columns="cols" :data="items" size="small" :bordered="false" :loading="loading" :pagination="{ pageSize: 15 }" />
  </div>
</template>

<script setup>
import { ref, h } from 'vue'
import { NButton, NSpace, NTag, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const items = ref([])
const dir = ref('')
const loading = ref(false)
const creating = ref(false)
const fileEl = ref(null)

function fmtSize(n) {
  if (n == null) return '-'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return n.toFixed(1) + ' ' + u[i]
}

const cols = [
  { title: '文件名', key: 'name', width: 280 },
  { title: '大小', key: 'size', width: 110, render: (r) => fmtSize(r.size) },
  { title: '创建时间', key: 'created', width: 180 },
  {
    title: '类型', key: 'kind', width: 100,
    render: (r) => h(NTag, { size: 'small', type: r.name.startsWith('upload-') ? 'warning' : 'info' }, { default: () => r.name.startsWith('upload-') ? '上传' : '本地' })
  },
  {
    title: '操作', key: 'act', width: 230,
    render: (r) => h(NSpace, { size: 'small' }, {
      default: () => [
        h(NButton, { size: 'tiny', onClick: () => download(r) }, { default: () => '下载' }),
        h(NButton, { size: 'tiny', type: 'warning', ghost: true, onClick: () => restore(r) }, { default: () => '恢复' }),
        h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => remove(r) }, { default: () => '删除' })
      ]
    })
  }
]

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/backup')
    items.value = data.items || []
    dir.value = data.dir || ''
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function create() {
  creating.value = true
  try {
    const { data } = await http.post('/backup')
    message.success(`已创建 ${data.name}（${fmtSize(data.size)}，${data.files} 个文件）`)
    await load()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { creating.value = false }
}

async function download(r) {
  try {
    const res = await http.get(`/backup/file/${encodeURIComponent(r.name)}`, { responseType: 'blob' })
    const url = URL.createObjectURL(new Blob([res.data]))
    const a = document.createElement('a')
    a.href = url
    a.download = r.name
    a.click()
    URL.revokeObjectURL(url)
  } catch (e) { message.error(String(errMsg(e))) }
}

async function onFile(e) {
  const f = e.target.files?.[0]
  if (!f) return
  const fd = new FormData()
  fd.append('file', f)
  try {
    const { data } = await http.post('/backup/upload', fd)
    message.success(`已上传 ${data.name}`)
    await load()
  } catch (err) { message.error(String(errMsg(err))) }
  e.target.value = ''
}

function restore(r) {
  dialog.warning({
    title: '确认恢复配置',
    content: `将用 ${r.name} 覆盖当前的防火墙 / DHCP / DNS 过滤 / 插件元数据等配置。当前管理员密码与登录会话会保留。建议先创建一份当前备份。`,
    positiveText: '恢复', negativeText: '取消',
    onPositiveClick: async () => {
      try {
        const { data } = await http.post('/backup/restore', { name: r.name })
        message.success(data.msg || '恢复完成')
      } catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

function remove(r) {
  dialog.warning({
    title: '删除备份', content: `确定删除 ${r.name}？`, positiveText: '删除', negativeText: '取消',
    onPositiveClick: async () => {
      try { await http.delete(`/backup/file/${encodeURIComponent(r.name)}`); message.success('已删除'); await load() }
      catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

load()
</script>
