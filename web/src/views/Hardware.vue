<template>
  <div>
    <n-tabs type="line">
      <n-tab-pane name="sensors" tab="温度 / 传感器">
        <div class="card-grid">
          <n-card size="small">
            <div class="muted">最高温度</div>
            <div :style="{ fontSize: '26px', fontWeight: 700, color: tempColor(maxTemp) }">{{ maxTemp.toFixed(1) }} °C</div>
            <div class="muted">{{ tempHint(maxTemp, tempCount) }}</div>
          </n-card>
          <n-card size="small"><div class="muted">温度传感器</div><div style="font-size: 26px; font-weight: 700">{{ tempCount }}</div></n-card>
          <n-card size="small"><div class="muted">风扇</div><div style="font-size: 26px; font-weight: 700">{{ fanCount }}</div><div class="muted">最高 {{ fanMax.toFixed(0) }} RPM</div></n-card>
          <n-card size="small"><div class="muted">传感器总数</div><div style="font-size: 26px; font-weight: 700">{{ sensors.length }}</div></n-card>
        </div>
        <div class="toolbar"><n-button @click="loadTemps" :loading="loadingTemps">刷新</n-button><span class="muted" style="color:#e88080">{{ tempMsg }}</span></div>
        <n-data-table :columns="sensorCols" :data="sensors" size="small" :bordered="false" />
      </n-tab-pane>

      <n-tab-pane name="leds" tab="LED 控制">
        <div class="toolbar"><n-button @click="loadLeds" :loading="loadingLeds">刷新</n-button><span class="muted" style="color:#e88080">{{ ledMsg }}</span></div>
        <n-alert type="info" size="small" style="margin-bottom: 12px">写入 /sys/class/leds 需要 root。亮度取值受 max_brightness 限制；trigger 只能从内核提供的可选值中选择。</n-alert>
        <n-data-table :columns="ledCols" :data="leds" size="small" :bordered="false" />
      </n-tab-pane>

      <n-tab-pane name="summary" tab="系统概览">
        <div class="toolbar"><n-button @click="loadSummary" :loading="loadingSummary">刷新</n-button></div>
        <n-grid :x-gap="12" :cols="3">
          <n-grid-item><n-card title="CPU" size="small">
            <div class="mono">平均频率：{{ summary.cpu?.avg_mhz ?? '-' }} MHz</div>
            <div class="mono">可读核心：{{ summary.cpu?.cores_with_freq ?? 0 }}</div>
            <div class="mono">负载：{{ summary.cpu?.loadavg || '-' }}</div>
          </n-card></n-grid-item>
          <n-grid-item><n-card title="运行时间" size="small">
            <div class="mono">{{ fmtUptime(summary.cpu?.uptime_secs) }}</div>
            <div class="mono" style="margin-top: 6px">LED 数量：{{ summary.led_count ?? 0 }}</div>
            <div class="mono">风扇数量：{{ summary.fan_count ?? 0 }}</div>
          </n-card></n-grid-item>
          <n-grid-item><n-card title="温度" size="small">
            <div class="mono" :style="{ color: tempColor(summary.temp_max) }">最高 {{ (summary.temp_max ?? 0).toFixed(1) }} °C</div>
            <div class="mono">温度点：{{ summary.temp_count ?? 0 }}</div>
            <div class="mono">传感器：{{ summary.sensor_count ?? 0 }}</div>
          </n-card></n-grid-item>
        </n-grid>
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, computed, h, onMounted } from 'vue'
import { NTag, NButton, NSpace, NInputNumber, NSelect, useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const sensors = ref([])
const leds = ref([])
const summary = ref({ cpu: {} })
const tempMsg = ref('')
const ledMsg = ref('')
const loadingTemps = ref(false)
const loadingLeds = ref(false)
const loadingSummary = ref(false)

const tempCount = computed(() => sensors.value.filter((x) => x.kind === 'temp').length)
const fanCount = computed(() => sensors.value.filter((x) => x.kind === 'fan').length)
const fanMax = computed(() => sensors.value.filter((x) => x.kind === 'fan').reduce((m, x) => Math.max(m, x.value || 0), 0))
const maxTemp = computed(() => sensors.value.filter((x) => x.kind === 'temp').reduce((m, x) => Math.max(m, x.celsius ?? x.value ?? 0), 0))

function tempColor(t) { return t >= 80 ? '#e88080' : t >= 60 ? '#e8b339' : '#63e2b7' }
function tempHint(t, n) { return n === 0 ? '未发现温度传感器' : t >= 80 ? '温度偏高，请检查散热' : t >= 60 ? '温度较高' : '温度正常' }
function fmtUptime(s) {
  if (!s) return '-'
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60)
  return `${d} 天 ${h} 小时 ${m} 分钟`
}

const sensorCols = [
  { title: '来源', key: 'source', width: 100 },
  { title: '名称', key: 'name', width: 200, ellipsis: { tooltip: true } },
  { title: '标签', key: 'label', width: 200, ellipsis: { tooltip: true } },
  { title: '类型', key: 'kind', width: 90, render: (r) => h(NTag, { size: 'small' }, { default: () => ({ temp: '温度', fan: '风扇', in: '电压' })[r.kind] || r.kind }) },
  {
    title: '数值', key: 'value', width: 140,
    render: (r) => {
      const v = r.celsius ?? r.value ?? 0
      const txt = r.kind === 'temp' ? `${v.toFixed(1)} °C` : r.kind === 'fan' ? `${v.toFixed(0)} RPM` : `${v.toFixed(0)} mV`
      return h('span', { style: { color: r.kind === 'temp' ? tempColor(v) : undefined, fontWeight: 600 } }, txt)
    }
  },
  { title: '原始值', key: 'raw', width: 110 }
]
const ledCols = [
  { title: 'LED', key: 'name', width: 220, ellipsis: { tooltip: true } },
  { title: '当前亮度', key: 'brightness', width: 100 },
  { title: '最大亮度', key: 'max_brightness', width: 100 },
  { title: '当前 trigger', key: 'trigger', width: 130, render: (r) => h(NTag, { size: 'small' }, { default: () => r.trigger || 'none' }) },
  {
    title: '设置亮度', key: 'set', width: 160,
    render: (r) => h(NInputNumber, {
      size: 'small', value: r._brightness ?? r.brightness, min: 0, max: r.max_brightness || 1,
      onUpdateValue: (v) => { r._brightness = v }
    })
  },
  {
    title: '设置 trigger', key: 'trig', width: 180,
    render: (r) => h(NSelect, {
      size: 'small', value: r._trigger ?? r.trigger, clearable: true,
      options: (r.triggers || []).map((t) => ({ label: t, value: t })),
      onUpdateValue: (v) => { r._trigger = v }
    })
  },
  {
    title: '操作', key: 'act', width: 150,
    render: (r) => h(NSpace, { size: 'small' }, { default: () => [
      h(NButton, { size: 'tiny', type: 'primary', disabled: !r.writable, onClick: () => writeLed(r, { brightness: r._brightness ?? r.brightness }) }, { default: () => '写入亮度' }),
      h(NButton, { size: 'tiny', disabled: !r.writable || !r._trigger, onClick: () => writeLed(r, { trigger: r._trigger }) }, { default: () => '写入 trigger' })
    ] })
  }
]

async function loadTemps() {
  loadingTemps.value = true
  try {
    const { data } = await http.get('/hardware/temps')
    sensors.value = data.items || []
    tempMsg.value = data.msg || ''
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingTemps.value = false }
}
async function loadLeds() {
  loadingLeds.value = true
  try {
    const { data } = await http.get('/hardware/leds')
    leds.value = (data.items || []).map((l) => ({ ...l, _brightness: l.brightness, _trigger: null }))
    ledMsg.value = data.msg || ''
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingLeds.value = false }
}
async function loadSummary() {
  loadingSummary.value = true
  try { const { data } = await http.get('/hardware/summary'); summary.value = data }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingSummary.value = false }
}
async function writeLed(r, payload) {
  try {
    await http.put('/hardware/led', { name: r.name, ...payload })
    message.success(`${r.name} 已写入`)
    await loadLeds()
  } catch (e) { message.error(String(errMsg(e))) }
}

onMounted(() => { loadTemps(); loadLeds(); loadSummary() })
</script>
