<template>
  <div>
    <n-alert type="warning" size="small" style="margin-bottom: 12px" title="QoS / SQM 说明">
      出口整形使用 tc + cake（或 fq_codel / htb），下行整形通过 IFB 设备把入口流量重定向后限速。
      规则即时生效但<strong>重启后失效</strong>；应用与清除都会生成脚本存到配置目录，可作为开机脚本复用。
    </n-alert>
    <n-tabs type="line">
      <n-tab-pane name="status" tab="运行状态">
        <div class="card-grid">
          <n-card size="small"><div class="muted">tc / iproute2</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ st.tc ? '可用' : '缺失' }}</div></n-card>
          <n-card size="small"><div class="muted">sch_cake 内核模块</div><div :style="{ fontSize: '20px', fontWeight: 700, color: st.cake ? '#63e2b7' : '#e88080' }">{{ st.cake ? '已加载' : '未加载' }}</div></n-card>
          <n-card size="small"><div class="muted">ifb 模块</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ st.ifb ? '已加载' : '未加载' }}</div></n-card>
          <n-card size="small"><div class="muted">当前队列规则</div><div :style="{ fontSize: '20px', fontWeight: 700 }">{{ (st.qdiscs || []).length }}</div></n-card>
        </div>
        <div class="toolbar"><n-button @click="loadStatus" :loading="loadingStatus">刷新</n-button><span class="muted" style="color:#e88080">{{ st.msg }}</span></div>
        <n-card title="qdisc 列表" size="small">
          <n-data-table :columns="qdiscCols" :data="st.qdiscs || []" size="small" :bordered="false" />
        </n-card>
        <pre class="code-block mono" style="margin-top: 12px; max-height: 260px">{{ st.stats || '（无统计输出）' }}</pre>
      </n-tab-pane>

      <n-tab-pane name="config" tab="规则配置">
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="编辑接口规则" size="small">
              <n-form label-placement="left" label-width="90" size="small">
                <n-form-item label="总开关"><n-switch v-model:value="cfg.enabled" /></n-form-item>
                <n-form-item label="接口"><n-select v-model:value="edit.iface" :options="ifaceOptions" filterable tag placeholder="如 eth0 / wan0" /></n-form-item>
                <n-form-item label="启用"><n-switch v-model:value="edit.enabled" /></n-form-item>
                <n-form-item label="下行 kbps"><n-input-number v-model:value="edit.download" :min="0" :max="10000000" :step="1000" /></n-form-item>
                <n-form-item label="上行 kbps"><n-input-number v-model:value="edit.upload" :min="0" :max="10000000" :step="1000" /></n-form-item>
                <n-form-item label="队列算法"><n-select v-model:value="edit.qdisc" :options="qdiscOpts" /></n-form-item>
                <n-form-item label="Diffserv"><n-select v-model:value="edit.diffserv" :options="diffservOpts" /></n-form-item>
                <n-form-item label="报文开销"><n-input-number v-model:value="edit.overhead" :min="0" :max="200" /></n-form-item>
              </n-form>
              <div class="toolbar" style="margin-bottom: 0">
                <n-button @click="upsertRule">{{ editIndex >= 0 ? '更新规则' : '添加规则' }}</n-button>
                <n-button v-if="editIndex >= 0" @click="resetEdit">取消编辑</n-button>
                <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
              </div>
              <div class="muted" style="margin-top: 8px">fq_codel 仅做队列管理、不限速；htb 限速仅作用于出口。建议上下行填真实带宽的 90%~95%。</div>
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="已配置规则" size="small">
              <n-data-table :columns="ruleCols" :data="cfg.ifaces || []" size="small" :bordered="false" />
              <div v-if="!(cfg.ifaces || []).length" class="muted" style="margin-top: 10px">暂无规则，添加一条后保存</div>
            </n-card>
          </n-grid-item>
        </n-grid>
      </n-tab-pane>

      <n-tab-pane name="apply" tab="应用与预览">
        <div class="toolbar">
          <n-button type="warning" @click="doApply" :loading="applying">应用 QoS 规则</n-button>
          <n-button @click="doClear" :loading="clearing">清除所有队列</n-button>
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
import { ref, computed, h, onMounted } from 'vue'
import { NTag, NButton, NSpace, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const st = ref({ qdiscs: [] })
const cfg = ref({ enabled: false, ifaces: [] })
const plan = ref([])
const steps = ref([])
const execMsg = ref('')
const execOk = ref(true)
const loadingStatus = ref(false)
const loadingCfg = ref(false)
const saving = ref(false)
const applying = ref(false)
const clearing = ref(false)
const editIndex = ref(-1)
const blank = () => ({ iface: 'eth0', enabled: true, download: 100000, upload: 30000, qdisc: 'cake', overhead: 34, diffserv: 'diffserv4' })
const edit = ref(blank())
const qdiscOpts = [{ label: 'cake（推荐，双向限速）', value: 'cake' }, { label: 'fq_codel（仅队列管理）', value: 'fq_codel' }, { label: 'htb（出口限速）', value: 'htb' }]
const diffservOpts = ['diffserv3', 'diffserv4', 'diffserv8', 'besteffort'].map((v) => ({ label: v, value: v }))

const ifaceOptions = computed(() => {
  const s = new Set(['eth0', 'eth1', 'wan0', 'ens18'])
  ;(cfg.value.ifaces || []).forEach((i) => i.iface && s.add(i.iface))
  if (edit.value.iface) s.add(edit.value.iface)
  return [...s].map((v) => ({ label: v, value: v }))
})

const qdiscCols = [
  { title: '类型', key: 'kind', width: 140 },
  { title: '句柄', key: 'handle', width: 100 },
  { title: '设备', key: 'dev', width: 130 },
  { title: '原始行', key: 'raw', ellipsis: { tooltip: true } }
]
const ruleCols = [
  { title: '接口', key: 'iface', width: 110 },
  { title: '启用', key: 'enabled', width: 80, render: (r) => h(NTag, { size: 'small', type: r.enabled ? 'success' : 'default' }, { default: () => r.enabled ? '是' : '否' }) },
  { title: '下行', key: 'download', width: 110, render: (r) => fmtRate(r.download) },
  { title: '上行', key: 'upload', width: 110, render: (r) => fmtRate(r.upload) },
  { title: '算法', key: 'qdisc', width: 110 },
  { title: '开销', key: 'overhead', width: 70 },
  {
    title: '操作', key: 'act', width: 130,
    render: (r, i) => h(NSpace, { size: 'small' }, { default: () => [
      h(NButton, { size: 'tiny', onClick: () => openEdit(r, i) }, { default: () => '编辑' }),
      h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => delRule(i) }, { default: () => '删除' })
    ] })
  }
]
const stepCols = [
  { title: '#', key: 'idx', width: 50, render: (_, i) => i + 1 },
  { title: '说明', key: 'note', width: 190 },
  { title: '可忽略失败', key: 'optional', width: 100, render: (r) => h(NTag, { size: 'small', type: r.optional ? 'default' : 'warning' }, { default: () => r.optional ? '是' : '否' }) },
  { title: '命令', key: 'cmd', render: (r) => h('code', { class: 'mono' }, r.cmd) }
]
const resultCols = [
  { title: '命令', key: 'cmd', ellipsis: { tooltip: true } },
  { title: '结果', key: 'ok', width: 90, render: (r) => h(NTag, { size: 'small', type: r.ok ? 'success' : 'error' }, { default: () => r.ok ? '成功' : '失败' }) },
  { title: '说明', key: 'note', width: 170 },
  { title: '错误', key: 'error', ellipsis: { tooltip: true } }
]

function fmtRate(k) { return k >= 1000 ? (k / 1000).toFixed(1) + ' Mbps' : k + ' kbps' }

async function loadStatus() {
  loadingStatus.value = true
  try { const { data } = await http.get('/qos/status'); st.value = data }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingStatus.value = false }
}

async function loadConfig() {
  loadingCfg.value = true
  try {
    const { data } = await http.get('/qos/config')
    cfg.value = data.config
    plan.value = data.plan || []
    qdiscOpts.splice(0, qdiscOpts.length, ...(data.qdiscs || ['cake', 'fq_codel', 'htb']).map((v) => ({ label: v, value: v })))
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingCfg.value = false }
}

function upsertRule() {
  if (!edit.value.iface) { message.warning('请填写接口名'); return }
  const item = { ...edit.value }
  const arr = [...(cfg.value.ifaces || [])]
  const dup = arr.findIndex((x, i) => x.iface === item.iface && i !== editIndex.value)
  if (dup >= 0) { message.warning('同一接口已有规则'); return }
  if (editIndex.value >= 0) arr[editIndex.value] = item
  else arr.push(item)
  cfg.value.ifaces = arr
  resetEdit()
}
function openEdit(r, i) { edit.value = { ...r }; editIndex.value = i }
function resetEdit() { edit.value = blank(); editIndex.value = -1 }
function delRule(i) { cfg.value.ifaces.splice(i, 1); resetEdit() }

async function save() {
  saving.value = true
  try {
    const { data } = await http.put('/qos/config', cfg.value)
    plan.value = data.plan || []
    message.success('配置已保存（尚未应用）')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

function doApply() {
  dialog.warning({
    title: '应用 QoS 规则', content: '将执行一系列 tc / ip 命令，会短暂影响该接口的流量。建议先确认带宽参数正确。',
    positiveText: '我已确认，执行', negativeText: '取消',
    onPositiveClick: async () => {
      applying.value = true
      try {
        await save()
        const { data } = await http.post('/qos/apply')
        steps.value = data.steps || []
        execMsg.value = data.msg || '已应用'
        execOk.value = true
        await loadStatus()
      } catch (e) {
        execMsg.value = String(errMsg(e)); execOk.value = false
      } finally { applying.value = false }
    }
  })
}

async function doClear() {
  clearing.value = true
  try {
    const { data } = await http.post('/qos/clear')
    steps.value = data.steps || []
    execMsg.value = data.msg
    execOk.value = true
    await loadStatus()
  } catch (e) { execMsg.value = String(errMsg(e)); execOk.value = false }
  finally { clearing.value = false }
}

onMounted(() => { loadStatus(); loadConfig() })
</script>
