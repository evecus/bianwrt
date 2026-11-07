<template>
  <div>
    <n-alert type="info" size="small" style="margin-bottom: 12px" title="dnsmasq 增强">
      规则写入 /etc/dnsmasq.d/bianwrt-filter.conf，与 DHCP/DNS 页面的 bianwrt.conf 相互独立。
      静态域名用 address=/域名/IP（自动匹配子域），拦截域名返回 0.0.0.0 与 ::，分流规则用 server=/域名/上游。
    </n-alert>
    <div class="toolbar">
      <n-switch v-model:value="cfg.enabled" /> <span class="muted">启用规则</span>
      <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
      <n-button type="warning" @click="apply" :loading="applying">应用到 dnsmasq</n-button>
      <n-button @click="load" :loading="loading">重新读取</n-button>
      <n-select v-model:value="preset" :options="presetOptions" placeholder="选择拦截预设" style="width: 200px" clearable />
      <n-button @click="mergePreset" :disabled="!preset">合并预设</n-button>
      <span class="muted">共 {{ cfg.hosts.length + cfg.blocked.length + cfg.forwards.length }} 条规则</span>
    </div>
    <n-tabs type="line">
      <n-tab-pane name="hosts" tab="静态域名">
        <div class="toolbar">
          <n-input v-model:value="hostForm.domain" placeholder="域名，如 router.lan" style="width: 240px" />
          <n-input v-model:value="hostForm.ip" placeholder="IP，如 192.168.1.1" style="width: 200px" />
          <n-button @click="addHost">添加</n-button>
        </div>
        <n-data-table :columns="hostCols" :data="cfg.hosts" size="small" :bordered="false" />
      </n-tab-pane>
      <n-tab-pane name="blocked" tab="域名拦截（广告过滤）">
        <div class="toolbar">
          <n-input v-model:value="blockInput" placeholder="每行一个域名，可批量粘贴" style="width: 320px" />
          <n-button @click="addBlocked">添加</n-button>
          <n-input v-model:value="blockForm" placeholder="单个域名，如 ads.example.com" style="width: 260px" />
          <n-button @click="addBlockedOne">添加单个</n-button>
        </div>
        <n-data-table :columns="blockCols" :data="blockRows" size="small" :bordered="false" :pagination="{ pageSize: 15 }" />
      </n-tab-pane>
      <n-tab-pane name="forwards" tab="按域名分流">
        <div class="toolbar">
          <n-input v-model:value="fwdForm.domain" placeholder="域名后缀，如 corp.local" style="width: 240px" />
          <n-input v-model:value="fwdForm.server" placeholder="上游 DNS，如 192.168.1.53 或 127.0.0.1#5353" style="width: 260px" />
          <n-button @click="addForward">添加</n-button>
        </div>
        <n-data-table :columns="fwdCols" :data="cfg.forwards" size="small" :bordered="false" />
      </n-tab-pane>
      <n-tab-pane name="preview" tab="生成的配置">
        <pre class="code-block mono" style="min-height: 260px">{{ preview }}</pre>
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, computed, h } from 'vue'
import { NButton, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const cfg = ref({ enabled: false, hosts: [], blocked: [], forwards: [] })
const presets = ref([])
const preset = ref(null)
const preview = ref('')
const loading = ref(false)
const saving = ref(false)
const applying = ref(false)
const hostForm = ref({ domain: '', ip: '' })
const fwdForm = ref({ domain: '', server: '' })
const blockInput = ref('')
const blockForm = ref('')

const presetOptions = computed(() => presets.value.map((p, i) => ({ label: `${p.name} (${(p.domains || []).length} 条)`, value: i })))
const blockRows = computed(() => cfg.value.blocked.map((d, i) => ({ idx: i, domain: d })))

const delCol = (onClick) => ({
  title: '操作', key: 'act', width: 90,
  render: (r) => h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => onClick(r) }, { default: () => '删除' })
})

const hostCols = [
  { title: '域名', key: 'domain' },
  { title: '指向 IP', key: 'ip', width: 220 },
  delCol((r) => { cfg.value.hosts = cfg.value.hosts.filter((x) => x !== r) })
]
const blockCols = [
  { title: '域名', key: 'domain' },
  delCol((r) => cfg.value.blocked.splice(r.idx, 1))
]
const fwdCols = [
  { title: '域名', key: 'domain' },
  { title: '上游 DNS', key: 'server', width: 260 },
  delCol((r) => { cfg.value.forwards = cfg.value.forwards.filter((x) => x !== r) })
]

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/dnsfilter')
    cfg.value = data.config || { enabled: false, hosts: [], blocked: [], forwards: [] }
    cfg.value.hosts ||= []
    cfg.value.blocked ||= []
    cfg.value.forwards ||= []
    presets.value = data.presets || []
    preview.value = data.preview || ''
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

function addHost() {
  if (!hostForm.value.domain || !hostForm.value.ip) { message.warning('请填写域名与 IP'); return }
  cfg.value.hosts.push({ domain: hostForm.value.domain.trim(), ip: hostForm.value.ip.trim() })
  hostForm.value = { domain: '', ip: '' }
}

function addBlockedOne() {
  if (!blockForm.value.trim()) return
  cfg.value.blocked.push(blockForm.value.trim())
  blockForm.value = ''
}

function addBlocked() {
  const lines = blockInput.value.split('\n').map((s) => s.trim()).filter((s) => s && !s.startsWith('#'))
  cfg.value.blocked.push(...lines)
  blockInput.value = ''
  message.success(`已添加 ${lines.length} 条`)
}

function addForward() {
  if (!fwdForm.value.domain || !fwdForm.value.server) { message.warning('请填写域名与上游 DNS'); return }
  cfg.value.forwards.push({ domain: fwdForm.value.domain.trim(), server: fwdForm.value.server.trim() })
  fwdForm.value = { domain: '', server: '' }
}

function mergePreset() {
  const p = presets.value[preset.value]
  if (!p) return
  const set = new Set(cfg.value.blocked)
  ;(p.domains || []).forEach((d) => set.add(d))
  cfg.value.blocked = [...set]
  message.success(`已合并「${p.name}」，当前共 ${cfg.value.blocked.length} 条`)
  preset.value = null
}

async function save() {
  saving.value = true
  try {
    const { data } = await http.put('/dnsfilter', cfg.value)
    preview.value = data.preview || ''
    message.success('配置已保存，点击「应用到 dnsmasq」使其生效')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

function apply() {
  dialog.warning({
    title: '应用 DNS 规则',
    content: '将写入 /etc/dnsmasq.d/bianwrt-filter.conf 并重启 dnsmasq，DNS 解析会短暂中断。',
    positiveText: '应用', negativeText: '取消',
    onPositiveClick: async () => {
      applying.value = true
      try {
        const { data } = await http.post('/dnsfilter/apply')
        preview.value = data.config || ''
        message.success('dnsmasq 已重启，规则生效')
      } catch (e) { message.error(String(errMsg(e))) }
      finally { applying.value = false }
    }
  })
}

load()
</script>
