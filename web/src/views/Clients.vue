<template>
  <div>
    <div class="card-grid">
      <n-card size="small"><n-statistic label="已发现主机" :value="stats.total" /></n-card>
      <n-card size="small"><n-statistic label="在线邻居" :value="stats.online" /></n-card>
      <n-card size="small"><n-statistic label="DHCP 租约" :value="stats.leases" /></n-card>
      <n-card size="small"><n-statistic label="静态绑定" :value="stats.statics" /></n-card>
    </div>
    <n-tabs type="line">
      <n-tab-pane name="hosts" tab="在线主机">
        <div class="toolbar">
          <n-input v-model:value="kw" placeholder="搜索 IP / MAC / 主机名" clearable style="width: 250px" />
          <n-button @click="load" :loading="loading">刷新</n-button>
          <n-input v-model:value="wolMac" placeholder="手动唤醒 MAC" style="width: 200px" />
          <n-button @click="wake(wolMac)" :disabled="!wolMac">发送唤醒包</n-button>
          <span class="muted">数据来自 ARP/ND 邻居表、dnsmasq 租约与静态绑定</span>
        </div>
        <n-data-table
          :columns="cols" :data="filtered" size="small" :bordered="false"
          :pagination="{ pageSize: 20 }" :row-key="(r) => r.ip" :max-height="'calc(100vh - 300px)'"
        />
      </n-tab-pane>
      <n-tab-pane name="conn" tab="连接跟踪">
        <div class="toolbar">
          <n-button @click="loadConn" :loading="connLoading">刷新</n-button>
          <span class="muted">{{ connMsg }}</span>
        </div>
        <n-data-table :columns="connCols" :data="connItems" size="small" :bordered="false" :pagination="{ pageSize: 20 }" />
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, computed, h } from 'vue'
import { NTag, NButton, useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const items = ref([])
const stats = ref({ total: 0, online: 0, leases: 0, statics: 0 })
const kw = ref('')
const loading = ref(false)
const wolMac = ref('')
const connItems = ref([])
const connMsg = ref('')
const connLoading = ref(false)

const filtered = computed(() => {
  const k = kw.value.trim().toLowerCase()
  if (!k) return items.value
  return items.value.filter((r) => [r.ip, r.mac, r.hostname, r.dev].join(' ').toLowerCase().includes(k))
})

const cols = [
  { title: 'IP 地址', key: 'ip', width: 160 },
  { title: 'MAC', key: 'mac', width: 180, render: (r) => r.mac || '-' },
  { title: '主机名', key: 'hostname', width: 170, render: (r) => r.hostname || '-' },
  { title: '接口', key: 'dev', width: 110, render: (r) => r.dev || '-' },
  {
    title: '状态', key: 'state', width: 150,
    render: (r) => h(NTag, { size: 'small', type: r.state === 'REACHABLE' || r.state === 'STALE' ? 'success' : 'default' }, { default: () => r.state || '-' })
  },
  {
    title: '来源', key: 'src', width: 160,
    render: (r) => h('div', { class: 'row-actions' }, [
      r.static ? h(NTag, { size: 'small', type: 'info' }, { default: () => '静态' }) : null,
      r.lease ? h(NTag, { size: 'small' }, { default: () => 'DHCP' }) : null,
      r.online && !r.lease && !r.static ? h(NTag, { size: 'small', type: 'success' }, { default: () => '邻居' }) : null
    ])
  },
  {
    title: '操作', key: 'act', width: 110,
    render: (r) => h(NButton, {
      size: 'tiny', disabled: !r.mac,
      onClick: () => wake(r.mac)
    }, { default: () => '网络唤醒' })
  }
]

const connCols = [
  { title: 'IP 地址', key: 'ip' },
  { title: '连接数', key: 'count', sorter: (a, b) => a.count - b.count }
]

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/clients')
    items.value = data.items || []
    stats.value = { total: data.total || 0, online: data.online || 0, leases: data.leases || 0, statics: data.statics || 0 }
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function loadConn() {
  connLoading.value = true
  try {
    const { data } = await http.get('/clients/connections')
    connItems.value = data.items || []
    connMsg.value = data.available ? `当前连接总数 ${data.total}` : (data.msg || '')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { connLoading.value = false }
}

async function wake(mac) {
  if (!mac) return
  try {
    const { data } = await http.post('/clients/wol', { mac })
    message.success(data.msg || '已发送')
  } catch (e) { message.error(String(errMsg(e))) }
}

load()
loadConn()
</script>
