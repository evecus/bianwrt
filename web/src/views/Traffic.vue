<template>
  <div>
    <n-alert type="info" size="small" style="margin-bottom: 12px" title="流量统计">
      数据来自 /proc/net/dev，每 5 秒采样一次，服务端保留最近 1 小时并每分钟落盘。
      「全部接口」为除 lo 外的速率求和；切换接口或时间范围会重新加载历史。
    </n-alert>
    <div class="toolbar">
      <n-select v-model:value="iface" :options="ifaceOptions" style="width: 190px" @update:value="loadHistory" />
      <n-select v-model:value="range" :options="ranges" style="width: 140px" @update:value="loadHistory" />
      <n-button @click="loadHistory" :loading="loading">刷新</n-button>
      <n-switch v-model:value="liveOn"><template #checked>实时刷新中</template><template #unchecked>实时刷新已暂停</template></n-switch>
      <span class="muted">采样间隔 {{ interval }}s · 已缓存 {{ points.length }} 点</span>
    </div>

    <div class="card-grid">
      <n-card size="small"><div class="muted">当前接收</div><div style="font-size: 22px; font-weight: 700; color: #63e2b7">{{ fmtSpeed(live.rx_bps) }}</div></n-card>
      <n-card size="small"><div class="muted">当前发送</div><div style="font-size: 22px; font-weight: 700; color: #e8b339">{{ fmtSpeed(live.tx_bps) }}</div></n-card>
      <n-card size="small"><div class="muted">峰值接收 / 发送</div><div style="font-size: 18px; font-weight: 700">{{ fmtSpeed(peak.rx) }} / {{ fmtSpeed(peak.tx) }}</div></n-card>
      <n-card size="small"><div class="muted">累计接收 / 发送</div><div style="font-size: 18px; font-weight: 700">{{ fmtBytes(live.rx_total) }} / {{ fmtBytes(live.tx_total) }}</div></n-card>
    </div>

    <n-card title="流量趋势" size="small">
      <div ref="chartEl" style="height: 340px; width: 100%"></div>
      <div v-if="!points.length" class="muted" style="text-align: center; padding: 8px">暂无采样数据，服务启动 5 秒后将开始记录</div>
    </n-card>

    <n-card title="各接口累计流量" size="small" style="margin-top: 12px">
      <div class="toolbar"><n-button @click="loadIfaces" size="small">刷新</n-button></div>
      <n-data-table :columns="ifaceCols" :data="ifaces" size="small" :bordered="false" />
    </n-card>
  </div>
</template>

<script setup>
import { ref, computed, h, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { NButton, useMessage } from 'naive-ui'
import * as echarts from 'echarts/core'
import { LineChart } from 'echarts/charts'
import { GridComponent, TooltipComponent, LegendComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import http, { errMsg } from '../lib/http'

echarts.use([LineChart, GridComponent, TooltipComponent, LegendComponent, CanvasRenderer])

const message = useMessage()
const chartEl = ref(null)
const iface = ref('__all__')
const range = ref('30m')
const liveOn = ref(true)
const loading = ref(false)
const interval = ref(5)
const points = ref([])
const live = ref({ rx_bps: 0, tx_bps: 0, rx_total: 0, tx_total: 0 })
const peak = ref({ rx: 0, tx: 0 })
const ifaces = ref([])
const ranges = [
  { label: '最近 10 分钟', value: '10m' },
  { label: '最近 30 分钟', value: '30m' },
  { label: '最近 1 小时', value: '1h' }
]
const minutes = { '10m': 10, '30m': 30, '1h': 60 }
const ifaceOptions = computed(() => [
  { label: '全部接口（汇总）', value: '__all__' },
  ...ifaces.value.map((i) => ({ label: i.name, value: i.name }))
])
const ifaceCols = [
  { title: '接口', key: 'name', width: 150 },
  { title: '当前接收', key: 'rx_bps', width: 150, render: (r) => fmtSpeed(r.rx_bps) },
  { title: '当前发送', key: 'tx_bps', width: 150, render: (r) => fmtSpeed(r.tx_bps) },
  { title: '累计接收', key: 'rx_total', width: 150, render: (r) => fmtBytes(r.rx_total) },
  { title: '累计发送', key: 'tx_total', width: 150, render: (r) => fmtBytes(r.tx_total) },
  { title: '操作', key: 'act', width: 100, render: (r) => h(NButton, { size: 'tiny', onClick: () => { iface.value = r.name; loadHistory() } }, { default: () => '查看趋势' }) }
]

function fmtBytes(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return n.toFixed(1) + ' ' + u[i]
}
function fmtSpeed(bps) {
  const bits = (bps || 0) * 8
  if (bits >= 1e9) return (bits / 1e9).toFixed(2) + ' Gbps'
  if (bits >= 1e6) return (bits / 1e6).toFixed(2) + ' Mbps'
  if (bits >= 1e3) return (bits / 1e3).toFixed(1) + ' Kbps'
  return bits.toFixed(0) + ' bps'
}
function mbps(bps) { return +(((bps || 0) * 8) / 1e6).toFixed(3) }
function hhmmss(ts) {
  const d = new Date(ts * 1000)
  return d.toLocaleTimeString('zh-CN', { hour12: false })
}

let chart = null
function render() {
  if (!chart) return
  const labels = points.value.map((p) => hhmmss(p.ts))
  const rx = points.value.map((p) => mbps(p.rx))
  const tx = points.value.map((p) => mbps(p.tx))
  chart.setOption({
    backgroundColor: 'transparent',
    animation: false,
    grid: { left: 62, right: 18, top: 36, bottom: 32 },
    tooltip: {
      trigger: 'axis',
      backgroundColor: '#1e2128', borderColor: '#3a3f4b', textStyle: { color: '#d3d8e0' },
      formatter: (ps) => {
        const t = ps[0]?.axisValue || ''
        return `${t}<br/>` + ps.map((p) => `${p.marker} ${p.seriesName}: <b>${p.value} Mbps</b>`).join('<br/>')
      }
    },
    legend: { data: ['接收', '发送'], textStyle: { color: '#9aa1ad' }, top: 4 },
    xAxis: {
      type: 'category', data: labels, boundaryGap: false,
      axisLabel: { color: '#8a909b', fontSize: 11 },
      axisLine: { lineStyle: { color: '#3a3f4b' } }, axisTick: { show: false }
    },
    yAxis: {
      type: 'value', name: 'Mbps', nameTextStyle: { color: '#8a909b' },
      axisLabel: { color: '#8a909b', fontSize: 11 },
      axisLine: { show: false }, splitLine: { lineStyle: { color: 'rgba(255,255,255,0.07)' } }
    },
    series: [
      {
        name: '接收', type: 'line', data: rx, smooth: true, showSymbol: false,
        lineStyle: { width: 2, color: '#63e2b7' }, itemStyle: { color: '#63e2b7' },
        areaStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
          { offset: 0, color: 'rgba(99,226,183,0.35)' }, { offset: 1, color: 'rgba(99,226,183,0.02)' }
        ]) }
      },
      {
        name: '发送', type: 'line', data: tx, smooth: true, showSymbol: false,
        lineStyle: { width: 2, color: '#e8b339' }, itemStyle: { color: '#e8b339' },
        areaStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
          { offset: 0, color: 'rgba(232,179,57,0.30)' }, { offset: 1, color: 'rgba(232,179,57,0.02)' }
        ]) }
      }
    ]
  }, true)
}

function params() {
  const p = { limit: minutes[range.value] * 60 / interval.value }
  if (iface.value !== '__all__') p.iface = iface.value
  return p
}

async function loadHistory() {
  loading.value = true
  try {
    const { data } = await http.get('/traffic/history', { params: params() })
    points.value = data.points || []
    peak.value = data.peak || { rx: 0, tx: 0 }
    if (data.interval) interval.value = data.interval
    render()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function loadIfaces() {
  try {
    const { data } = await http.get('/traffic/interfaces')
    ifaces.value = data.items || []
    if (data.interval) interval.value = data.interval
  } catch (e) { message.error(String(errMsg(e))) }
}

async function pollLive() {
  if (!liveOn.value) return
  try {
    const { data } = await http.get('/traffic/live', { params: iface.value === '__all__' ? {} : { iface: iface.value } })
    live.value = data
    points.value.push({ ts: data.ts, rx: data.rx_bps, tx: data.tx_bps })
    const max = minutes[range.value] * 60 / interval.value
    while (points.value.length > max) points.value.shift()
    peak.value = {
      rx: Math.max(peak.value.rx || 0, data.rx_bps),
      tx: Math.max(peak.value.tx || 0, data.tx_bps)
    }
    render()
  } catch { /* 静默，避免轮询刷屏 */ }
}

let timer = null
function onResize() { chart && chart.resize() }

onMounted(async () => {
  await nextTick()
  if (chartEl.value) chart = echarts.init(chartEl.value)
  window.addEventListener('resize', onResize)
  await loadIfaces()
  await loadHistory()
  await pollLive()
  timer = setInterval(() => { pollLive(); loadIfaces() }, 3000)
})
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
  window.removeEventListener('resize', onResize)
  if (chart) { chart.dispose(); chart = null }
})
</script>
