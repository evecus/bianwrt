<template>
  <div>
    <n-alert v-if="statusMsg" type="warning" size="small" style="margin-bottom: 12px">{{ statusMsg }}</n-alert>
    <n-tabs type="line">
      <n-tab-pane name="status" tab="运行状态">
        <div class="card-grid">
          <n-card size="small"><div class="muted">hostapd 服务</div><div :style="{ fontSize: '20px', fontWeight: 700, color: st.active ? '#63e2b7' : '#8a909b' }">{{ st.active ? '运行中' : '未运行' }}</div></n-card>
          <n-card size="small"><div class="muted">开机自启</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ st.boot_enabled ? '已启用' : '未启用' }}</div></n-card>
          <n-card size="small"><div class="muted">无线工具</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ st.iw ? 'iw ✓' : 'iw ✗' }} / {{ st.hostapd ? 'hostapd ✓' : 'hostapd ✗' }}</div></n-card>
          <n-card size="small"><div class="muted">配置文件</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ st.config_exists ? '已生成' : '未生成' }}</div></n-card>
        </div>
        <div class="toolbar">
          <n-button @click="loadStatus" :loading="loadingStatus">刷新</n-button>
          <n-button @click="act('start')">启动</n-button>
          <n-button @click="act('stop')">停止</n-button>
          <n-button @click="act('restart')">重启</n-button>
          <n-button @click="act('enable')">开机自启</n-button>
          <n-button @click="act('disable')">取消自启</n-button>
        </div>
        <n-card title="无线设备（iw dev）" size="small">
          <n-data-table :columns="devCols" :data="st.devices || []" size="small" :bordered="false" />
          <div v-if="!(st.devices || []).length" class="muted" style="margin-top: 8px">未发现无线设备</div>
        </n-card>
      </n-tab-pane>

      <n-tab-pane name="config" tab="AP 配置">
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="hostapd 参数" size="small">
              <n-form label-placement="left" label-width="100" size="small">
                <n-form-item label="启用 AP"><n-switch v-model:value="cfg.enabled" /></n-form-item>
                <n-form-item label="无线接口"><n-select v-model:value="cfg.iface" :options="ifaceOptions" filterable tag /></n-form-item>
                <n-form-item label="SSID"><n-input v-model:value="cfg.ssid" maxlength="32" /></n-form-item>
                <n-form-item label="密码"><n-input v-model:value="cfg.password" type="password" show-password-on="click" placeholder="留空为开放网络，8-63 字符" /></n-form-item>
                <n-form-item label="加密"><n-select v-model:value="cfg.wpa" :options="[{label:'WPA2-PSK',value:2},{label:'WPA3-SAE',value:3}]" /></n-form-item>
                <n-form-item label="频段"><n-select v-model:value="cfg.hw_mode" :options="hwModes" /></n-form-item>
                <n-form-item label="信道"><n-input-number v-model:value="cfg.channel" :min="1" :max="196" /></n-form-item>
                <n-form-item label="国家代码"><n-input v-model:value="cfg.country" maxlength="2" placeholder="CN" /></n-form-item>
                <n-form-item label="桥接到"><n-input v-model:value="cfg.bridge" placeholder="可选，如 br-lan" /></n-form-item>
                <n-form-item label="隐藏 SSID"><n-switch v-model:value="cfg.hidden" /></n-form-item>
                <n-form-item label="客户端隔离"><n-switch v-model:value="cfg.isolate" /></n-form-item>
                <n-form-item label="802.11n"><n-switch v-model:value="cfg.ieee80211n" /></n-form-item>
                <n-form-item label="802.11ac"><n-switch v-model:value="cfg.ieee80211ac" :disabled="cfg.hw_mode !== 'a'" /></n-form-item>
              </n-form>
              <div class="toolbar" style="margin-top: 6px; margin-bottom: 0">
                <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
                <n-button type="warning" @click="apply" :loading="applying">应用并重启 hostapd</n-button>
              </div>
              <div class="muted" style="margin-top: 8px">配置写入 /etc/hostapd/hostapd.conf（权限 600）。应用会重启服务，请确认无线网卡支持 AP 模式。</div>
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="hostapd.conf 预览" size="small">
              <pre class="code-block mono" style="min-height: 380px; max-height: 520px">{{ preview }}</pre>
            </n-card>
          </n-grid-item>
        </n-grid>
      </n-tab-pane>

      <n-tab-pane name="stations" tab="已连接站点">
        <div class="toolbar">
          <n-select v-model:value="selIface" :options="ifaceOptions" style="width: 180px" />
          <n-button @click="loadStations" :loading="loadingStations">刷新站点</n-button>
          <span class="muted">iw dev {{ selIface }} station dump</span>
        </div>
        <n-alert v-if="stationMsg" type="info" size="small" style="margin-bottom: 10px">{{ stationMsg }}</n-alert>
        <n-data-table :columns="stationCols" :data="stations" size="small" :bordered="false" />
      </n-tab-pane>

      <n-tab-pane name="scan" tab="周边扫描">
        <div class="toolbar">
          <n-select v-model:value="selIface" :options="ifaceOptions" style="width: 180px" />
          <n-button type="primary" @click="scan" :loading="scanning">扫描周边 AP</n-button>
          <span class="muted">iw dev {{ selIface }} scan（可能需要 root）</span>
        </div>
        <n-alert v-if="scanMsg" type="info" size="small" style="margin-bottom: 10px">{{ scanMsg }}</n-alert>
        <n-data-table :columns="scanCols" :data="apList" size="small" :bordered="false" />
      </n-tab-pane>

      <n-tab-pane name="cap" tab="硬件能力">
        <div class="toolbar"><n-button @click="loadCaps" :loading="loadingCaps">刷新 iw list</n-button><span class="muted">支持模式 / 频段</span></div>
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="Supported interface modes" size="small">
              <div v-for="m in caps.modes || []" :key="m" class="mono">· {{ m }}</div>
              <div v-if="!(caps.modes || []).length" class="muted">无数据</div>
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="Bands" size="small">
              <div v-for="b in caps.bands || []" :key="b" class="mono">· {{ b }}</div>
              <div v-if="!(caps.bands || []).length" class="muted">无数据</div>
            </n-card>
          </n-grid-item>
        </n-grid>
        <pre class="code-block mono" style="margin-top: 12px; max-height: 260px">{{ caps.raw }}</pre>
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, computed, h, onMounted } from 'vue'
import { NTag, NButton, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const st = ref({ devices: [] })
const statusMsg = ref('')
const cfg = ref({})
const preview = ref('')
const hwModes = ref([{ label: '2.4GHz (g)', value: 'g' }, { label: '5GHz (a)', value: 'a' }, { label: '2.4GHz (b)', value: 'b' }])
const selIface = ref('wlan0')
const stations = ref([])
const apList = ref([])
const caps = ref({ modes: [], bands: [], raw: '' })
const stationMsg = ref('')
const scanMsg = ref('')
const loadingStatus = ref(false)
const loadingStations = ref(false)
const scanning = ref(false)
const loadingCaps = ref(false)
const saving = ref(false)
const applying = ref(false)

const ifaceOptions = computed(() => {
  const s = new Set()
  ;(st.value.devices || []).forEach((d) => d.name && s.add(d.name))
  if (cfg.value.iface) s.add(cfg.value.iface)
  s.add('wlan0')
  return [...s].map((v) => ({ label: v, value: v }))
})

const devCols = [
  { title: '接口', key: 'name', width: 130 },
  { title: 'PHY', key: 'phy', width: 80 },
  { title: '模式', key: 'type', width: 110, render: (r) => h(NTag, { size: 'small', type: r.type === 'AP' ? 'success' : 'default' }, { default: () => r.type || '-' }) },
  { title: 'SSID', key: 'ssid', width: 150 },
  { title: '信道', key: 'channel', width: 140 },
  { title: 'MAC', key: 'mac', width: 170 },
  { title: '发射功率', key: 'txpower', width: 120 }
]
const stationCols = [
  { title: 'MAC', key: 'mac', width: 190 },
  { title: '信号', key: 'signal', width: 110 },
  { title: '空闲', key: 'inactive', width: 110 },
  { title: '接收', key: 'rx_bytes', width: 110, render: (r) => fmtBytes(r.rx_bytes) },
  { title: '发送', key: 'tx_bytes', width: 110, render: (r) => fmtBytes(r.tx_bytes) },
  { title: 'TX 速率', key: 'tx_bitrate', width: 150 },
  { title: 'RX 速率', key: 'rx_bitrate', width: 150 }
]
const scanCols = [
  { title: 'SSID', key: 'ssid', width: 200 },
  { title: 'BSSID', key: 'bssid', width: 190 },
  { title: '信号', key: 'signal', width: 110 },
  { title: '频率', key: 'freq', width: 100 },
  { title: '安全', key: 'security', width: 120, render: (r) => h(NTag, { size: 'small', type: r.security === '开放' ? 'warning' : 'info' }, { default: () => r.security }) }
]

function fmtBytes(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return n.toFixed(1) + ' ' + u[i]
}

async function loadStatus() {
  loadingStatus.value = true
  try {
    const { data } = await http.get('/wireless/status')
    st.value = data
    statusMsg.value = data.msg || ''
    if (!selIface.value && (data.devices || []).length) selIface.value = data.devices[0].name
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingStatus.value = false }
}

async function loadConfig() {
  try {
    const { data } = await http.get('/wireless/config')
    cfg.value = data.config
    preview.value = data.preview
    hwModes.value = data.hw_modes || hwModes.value
  } catch (e) { message.error(String(errMsg(e))) }
}

async function save() {
  saving.value = true
  try {
    const { data } = await http.put('/wireless/config', cfg.value)
    preview.value = data.preview
    message.success('配置已保存（尚未应用）')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

function apply() {
  dialog.warning({
    title: '应用 AP 配置', content: '将写入 /etc/hostapd/hostapd.conf 并重启 hostapd 服务，无线连接会短暂中断。',
    positiveText: '我已确认，执行', negativeText: '取消',
    onPositiveClick: async () => {
      applying.value = true
      try {
        await save()
        const { data } = await http.post('/wireless/apply')
        preview.value = data.config
        message.success(data.msg || '已应用')
        await loadStatus()
      } catch (e) { message.error(String(errMsg(e))) }
      finally { applying.value = false }
    }
  })
}

async function act(op) {
  try {
    const { data } = await http.post('/wireless/action', { op })
    message.success(data.msg || '操作成功')
    await loadStatus()
  } catch (e) { message.error(String(errMsg(e))) }
}

async function loadStations() {
  loadingStations.value = true
  try {
    const { data } = await http.get('/wireless/stations', { params: { iface: selIface.value } })
    stations.value = data.items || []
    stationMsg.value = data.msg || ''
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingStations.value = false }
}

async function scan() {
  scanning.value = true
  try {
    const { data } = await http.get('/wireless/scan', { params: { iface: selIface.value } })
    apList.value = data.items || []
    scanMsg.value = data.msg || `扫描到 ${apList.value.length} 个 AP`
  } catch (e) { message.error(String(errMsg(e))) }
  finally { scanning.value = false }
}

async function loadCaps() {
  loadingCaps.value = true
  try { const { data } = await http.get('/wireless/devices'); caps.value = data }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingCaps.value = false }
}

onMounted(() => { loadStatus(); loadConfig(); loadCaps() })
</script>
