<template>
  <div>
    <n-alert type="warning" size="small" style="margin-bottom: 12px" title="多 WAN / 策略路由提示">
      路由表 ID 限定 100-250、规则优先级限定 1000-1999（bianwrt 保留段），避免与系统规则冲突。
      应用会即时修改内核路由表，可能中断当前连接；「清除规则」只删除本配置引用的表与对应规则。
    </n-alert>
    <n-tabs type="line">
      <n-tab-pane name="status" tab="当前状态">
        <div class="toolbar"><n-button @click="loadStatus" :loading="loadingStatus">刷新</n-button><span class="muted">ip rule / ip route show table all</span></div>
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="策略规则 (ip rule)" size="small">
              <n-data-table :columns="ruleCols" :data="status.rules || []" size="small" :bordered="false" :max-height="300" />
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="接口与地址" size="small">
              <n-data-table :columns="ifaceCols" :data="status.ifaces || []" size="small" :bordered="false" :max-height="300" />
            </n-card>
          </n-grid-item>
        </n-grid>
        <n-card title="全部路由表" size="small" style="margin-top: 12px">
          <n-data-table :columns="routeCols" :data="status.routes || []" size="small" :bordered="false" :max-height="300" />
        </n-card>
      </n-tab-pane>

      <n-tab-pane name="config" tab="WAN 与策略">
        <n-card size="small" style="margin-bottom: 12px">
          <div class="toolbar" style="margin-bottom: 0">
            <span class="muted">总开关</span><n-switch v-model:value="cfg.enabled" />
            <span class="muted">模式</span><n-select v-model:value="cfg.mode" :options="modes" style="width: 220px" />
            <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
          </div>
        </n-card>
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="编辑 WAN 上行" size="small">
              <n-form label-placement="left" label-width="90" size="small">
                <n-form-item label="名称"><n-input v-model:value="wan.name" /></n-form-item>
                <n-form-item label="接口"><n-input v-model:value="wan.iface" placeholder="eth1 / pppoe0" /></n-form-item>
                <n-form-item label="网关"><n-input v-model:value="wan.gateway" placeholder="192.168.2.1" /></n-form-item>
                <n-form-item label="路由表 ID"><n-input-number v-model:value="wan.table" :min="100" :max="250" /></n-form-item>
                <n-form-item label="Metric"><n-input-number v-model:value="wan.metric" :min="0" :max="32767" /></n-form-item>
                <n-form-item label="权重"><n-input-number v-model:value="wan.weight" :min="1" :max="255" /></n-form-item>
                <n-form-item label="探测 IP"><n-input v-model:value="wan.probe" placeholder="223.5.5.5" /></n-form-item>
                <n-form-item label="启用"><n-switch v-model:value="wan.enabled" /></n-form-item>
              </n-form>
              <div class="toolbar" style="margin-bottom: 0">
                <n-button @click="upsertWan">{{ wanIndex >= 0 ? '更新 WAN' : '添加 WAN' }}</n-button>
                <n-button v-if="wanIndex >= 0" @click="resetWan">取消编辑</n-button>
              </div>
              <n-data-table :columns="wanCols" :data="cfg.wans || []" size="small" :bordered="false" style="margin-top: 10px" />
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="编辑策略规则" size="small">
              <n-form label-placement="left" label-width="90" size="small">
                <n-form-item label="名称"><n-input v-model:value="pol.name" /></n-form-item>
                <n-form-item label="源地址"><n-input v-model:value="pol.src" placeholder="192.168.1.0/24 或留空" /></n-form-item>
                <n-form-item label="目标地址"><n-input v-model:value="pol.dst" placeholder="可选 CIDR" /></n-form-item>
                <n-form-item label="查表 ID"><n-input-number v-model:value="pol.table" :min="100" :max="250" /></n-form-item>
                <n-form-item label="优先级"><n-input-number v-model:value="pol.priority" :min="1000" :max="1999" /></n-form-item>
                <n-form-item label="启用"><n-switch v-model:value="pol.enabled" /></n-form-item>
              </n-form>
              <div class="toolbar" style="margin-bottom: 0">
                <n-button @click="upsertPol">{{ polIndex >= 0 ? '更新策略' : '添加策略' }}</n-button>
                <n-button v-if="polIndex >= 0" @click="resetPol">取消编辑</n-button>
              </div>
              <n-data-table :columns="polCols" :data="cfg.policies || []" size="small" :bordered="false" style="margin-top: 10px" />
            </n-card>
          </n-grid-item>
        </n-grid>
      </n-tab-pane>

      <n-tab-pane name="apply" tab="应用与预览">
        <div class="toolbar">
          <n-button type="warning" @click="doApply" :loading="applying">应用策略路由</n-button>
          <n-button @click="doClear" :loading="clearing">清除规则</n-button>
          <n-button @click="loadConfig" :loading="loadingCfg">刷新预览</n-button>
          <span class="muted">共 {{ plan.length }} 条命令</span>
        </div>
        <n-alert v-if="execMsg" :type="execOk ? 'success' : 'error'" size="small" style="margin-bottom: 12px">{{ execMsg }}</n-alert>
        <n-data-table :columns="stepCols" :data="plan" size="small" :bordered="false" :max-height="360" />
        <div v-if="steps.length" class="section-title">最近一次执行结果</div>
        <n-data-table v-if="steps.length" :columns="resultCols" :data="steps" size="small" :bordered="false" :max-height="260" />
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, h, onMounted } from 'vue'
import { NTag, NButton, NSpace, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const status = ref({ rules: [], routes: [], main: [], ifaces: [] })
const cfg = ref({ enabled: false, mode: 'failover', wans: [], policies: [] })
const plan = ref([])
const steps = ref([])
const execMsg = ref('')
const execOk = ref(true)
const modes = ref([{ label: '故障切换 (failover)', value: 'failover' }, { label: '按权重负载均衡 (balance)', value: 'balance' }])
const loadingStatus = ref(false)
const loadingCfg = ref(false)
const saving = ref(false)
const applying = ref(false)
const clearing = ref(false)
const wanIndex = ref(-1)
const polIndex = ref(-1)
const blankWan = () => ({ name: 'wan1', iface: 'eth1', gateway: '', table: 101, metric: 10, weight: 1, probe: '223.5.5.5', enabled: true })
const blankPol = () => ({ name: 'lan-via-wan1', src: '192.168.1.0/24', dst: '', table: 101, priority: 1001, enabled: true })
const wan = ref(blankWan())
const pol = ref(blankPol())

const ruleCols = [
  { title: '优先级', key: 'priority', width: 100 },
  { title: '来源', key: 'src', width: 170 },
  { title: '目标', key: 'dst', width: 170 },
  { title: '查表', key: 'table', width: 110 },
  { title: '动作', key: 'action', width: 110 }
]
const ifaceCols = [
  { title: '接口', key: 'name', width: 150 },
  { title: 'IPv4', key: 'addr', width: 170 }
]
const routeCols = [
  { title: '目标', key: 'dst', width: 160 },
  { title: '下一跳', key: 'gateway', width: 150 },
  { title: '出接口', key: 'dev', width: 120 },
  { title: '表', key: 'table', width: 90 },
  { title: 'Metric', key: 'metric', width: 90 },
  { title: '协议', key: 'protocol', width: 100 }
]
const wanCols = [
  { title: '名称', key: 'name', width: 100 },
  { title: '接口', key: 'iface', width: 100 },
  { title: '网关', key: 'gateway', width: 140 },
  { title: '表', key: 'table', width: 60 },
  { title: 'Metric', key: 'metric', width: 80 },
  { title: '启用', key: 'enabled', width: 70, render: (r) => h(NTag, { size: 'small', type: r.enabled ? 'success' : 'default' }, { default: () => r.enabled ? '是' : '否' }) },
  { title: '操作', key: 'act', width: 120, render: (r, i) => h(NSpace, { size: 'small' }, { default: () => [
    h(NButton, { size: 'tiny', onClick: () => { wan.value = { ...r }; wanIndex.value = i } }, { default: () => '编辑' }),
    h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => cfg.value.wans.splice(i, 1) }, { default: () => '删除' })
  ] }) }
]
const polCols = [
  { title: '名称', key: 'name', width: 140 },
  { title: '源', key: 'src', width: 160 },
  { title: '目标', key: 'dst', width: 120 },
  { title: '表', key: 'table', width: 60 },
  { title: '优先级', key: 'priority', width: 80 },
  { title: '操作', key: 'act', width: 120, render: (r, i) => h(NSpace, { size: 'small' }, { default: () => [
    h(NButton, { size: 'tiny', onClick: () => { pol.value = { ...r }; polIndex.value = i } }, { default: () => '编辑' }),
    h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => cfg.value.policies.splice(i, 1) }, { default: () => '删除' })
  ] }) }
]
const stepCols = [
  { title: '#', key: 'idx', width: 50, render: (_, i) => i + 1 },
  { title: '说明', key: 'note', width: 190 },
  { title: '可忽略', key: 'optional', width: 80, render: (r) => h(NTag, { size: 'small', type: r.optional ? 'default' : 'warning' }, { default: () => r.optional ? '是' : '否' }) },
  { title: '命令', key: 'cmd', render: (r) => h('code', { class: 'mono' }, r.cmd) }
]
const resultCols = [
  { title: '命令', key: 'cmd', ellipsis: { tooltip: true } },
  { title: '结果', key: 'ok', width: 90, render: (r) => h(NTag, { size: 'small', type: r.ok ? 'success' : 'error' }, { default: () => r.ok ? '成功' : '失败' }) }
]

async function loadStatus() {
  loadingStatus.value = true
  try {
    const { data } = await http.get('/mwan/status')
    status.value = {
      rules: Array.isArray(data.rules) ? data.rules : [],
      routes: Array.isArray(data.routes) ? data.routes : [],
      main: Array.isArray(data.main) ? data.main : [],
      ifaces: data.ifaces || []
    }
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingStatus.value = false }
}

async function loadConfig() {
  loadingCfg.value = true
  try {
    const { data } = await http.get('/mwan/config')
    cfg.value = data.config
    plan.value = data.plan || []
    if (data.modes) modes.value = data.modes
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingCfg.value = false }
}

function upsertWan() {
  if (!wan.value.iface || !wan.value.gateway) { message.warning('请填写接口与网关'); return }
  const item = { ...wan.value }
  const arr = [...(cfg.value.wans || [])]
  if (editIndexDup(arr, 'table', item.table, wanIndex.value)) { message.warning('路由表 ID 重复'); return }
  if (wanIndex.value >= 0) arr[wanIndex.value] = item; else arr.push(item)
  cfg.value.wans = arr
  resetWan()
}
function editIndexDup(arr, key, val, skip) { return arr.findIndex((x, i) => x[key] === val && i !== skip) >= 0 }
function resetWan() { wan.value = blankWan(); wanIndex.value = -1 }
function upsertPol() {
  const item = { ...pol.value }
  const arr = [...(cfg.value.policies || [])]
  if (arr.findIndex((x, i) => x.priority === item.priority && i !== polIndex.value) >= 0) { message.warning('优先级重复'); return }
  if (polIndex.value >= 0) arr[polIndex.value] = item; else arr.push(item)
  cfg.value.policies = arr
  resetPol()
}
function resetPol() { pol.value = blankPol(); polIndex.value = -1 }

async function save() {
  saving.value = true
  try {
    const { data } = await http.put('/mwan/config', cfg.value)
    plan.value = data.plan || []
    message.success('配置已保存（尚未应用）')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

function doApply() {
  dialog.warning({
    title: '应用策略路由', content: '将即时修改内核路由表与规则，可能导致当前 SSH / 面板连接中断。请确认网关可达、有带外恢复方式。',
    positiveText: '我已确认，执行', negativeText: '取消',
    onPositiveClick: async () => {
      applying.value = true
      try {
        await save()
        const { data } = await http.post('/mwan/apply')
        steps.value = data.steps || []
        execMsg.value = data.msg || '已应用'; execOk.value = true
        await loadStatus()
      } catch (e) { execMsg.value = String(errMsg(e)); execOk.value = false }
      finally { applying.value = false }
    }
  })
}

async function doClear() {
  clearing.value = true
  try {
    const { data } = await http.post('/mwan/clear')
    steps.value = data.steps || []
    execMsg.value = data.msg + `（删除 ${data.removed_rules} 条规则）`
    execOk.value = true
    await loadStatus()
  } catch (e) { execMsg.value = String(errMsg(e)); execOk.value = false }
  finally { clearing.value = false }
}

onMounted(() => { loadStatus(); loadConfig() })
</script>
