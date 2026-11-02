<template>
  <n-spin :show="loading">
    <n-grid :x-gap="16" :y-gap="16" :cols="4">
      <n-grid-item>
        <n-card title="系统">
          <n-descriptions :column="1" size="small" label-placement="left">
            <n-descriptions-item label="主机名">{{ s.hostname }}</n-descriptions-item>
            <n-descriptions-item label="系统">{{ s.os }}</n-descriptions-item>
            <n-descriptions-item label="内核">{{ s.kernel }}</n-descriptions-item>
            <n-descriptions-item label="运行时长">{{ fmtUptime(s.uptime) }}</n-descriptions-item>
          </n-descriptions>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="CPU">
          <n-statistic :value="s.cpu?.usage?.toFixed(1) ?? '0'">
            <template #suffix>%</template>
          </n-statistic>
          <n-progress type="line" :percentage="s.cpu?.usage ?? 0" :show-indicator="false" />
          <div style="margin-top: 8px; color: #888; font-size: 12px">{{ s.cpu?.brand }} · {{ s.cpu?.cores }} 核</div>
          <div style="margin-top: 6px; color: #888; font-size: 12px">负载 {{ s.load?.['1'] }} / {{ s.load?.['5'] }} / {{ s.load?.['15'] }}</div>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="内存">
          <n-statistic :value="memPct.toFixed(1)">
            <template #suffix>%</template>
          </n-statistic>
          <n-progress type="line" :percentage="memPct" :show-indicator="false" />
          <div style="margin-top: 8px; color: #888; font-size: 12px">
            {{ fmtSize(s.mem?.used) }} / {{ fmtSize(s.mem?.total) }}
          </div>
          <div v-if="s.mem?.swap_total" style="margin-top: 4px; color: #888; font-size: 12px">
            Swap {{ fmtSize(s.mem?.swap_used) }} / {{ fmtSize(s.mem?.swap_total) }}
          </div>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="磁盘">
          <div v-for="d in s.disks || []" :key="d.mount" style="margin-bottom: 10px">
            <div style="font-size: 12px; color: #888">{{ d.mount }} ({{ d.fs }})</div>
            <n-progress type="line" :percentage="diskPct(d)" :height="8" :show-indicator="false" />
            <div style="font-size: 12px; color: #888">{{ fmtSize(d.total - d.free) }} / {{ fmtSize(d.total) }}</div>
          </div>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="网络流量">
          <n-data-table :columns="netCols" :data="s.net || []" size="small" :bordered="false" />
        </n-card>
      </n-grid-item>
    </n-grid>
  </n-spin>
</template>

<script setup>
import { ref, onMounted, onUnmounted, computed, h } from 'vue'
import { NCard, NGrid, NGridItem, NStatistic, NProgress, NDescriptions, NDescriptionsItem, NDataTable, NSpin, NTag } from 'naive-ui'
import http from '../lib/http'

const s = ref({})
const loading = ref(true)
let timer

const memPct = computed(() =>
  s.value.mem?.total ? (s.value.mem.used / s.value.mem.total) * 100 : 0
)

const netCols = [
  { title: '接口', key: 'name' },
  { title: '接收速率', key: 'rx_speed', render: (r) => fmtSpeed(r.rx_speed) },
  { title: '发送速率', key: 'tx_speed', render: (r) => fmtSpeed(r.tx_speed) },
  { title: '累计接收', key: 'rx_total', render: (r) => fmtSize(r.rx_total) },
  { title: '累计发送', key: 'tx_total', render: (r) => fmtSize(r.tx_total) }
]

function fmtSize(b) {
  if (b == null) return '-'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++ }
  return b.toFixed(1) + ' ' + u[i]
}
function fmtSpeed(bps) {
  if (bps == null) return '-'
  return fmtSize(bps) + '/s'
}
function fmtUptime(sec) {
  if (!sec) return '-'
  const d = Math.floor(sec / 86400)
  const h = Math.floor((sec % 86400) / 3600)
  const m = Math.floor((sec % 3600) / 60)
  return `${d} 天 ${h} 时 ${m} 分`
}
function diskPct(d) {
  return d.total ? ((d.total - d.free) / d.total) * 100 : 0
}

async function load() {
  try {
    const { data } = await http.get('/system/stats')
    s.value = data
  } finally { loading.value = false }
}
onMounted(() => {
  load()
  timer = setInterval(load, 3000)
})
onUnmounted(() => clearInterval(timer))
</script>
