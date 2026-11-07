<template>
  <div>
    <n-alert type="info" size="small" style="margin-bottom: 12px" title="DDNS 工作方式">
      公网 IP 来源于外部服务、指定接口或手动填写；后台任务每 60 秒检查一次，按每个条目的间隔自动更新。
      密钥在列表中脱敏，编辑时留空表示保持原值。更新 URL 以单个参数传给 curl，不经过 shell。
    </n-alert>
    <n-tabs type="line">
      <n-tab-pane name="config" tab="DDNS 配置">
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="编辑条目" size="small">
              <n-form label-placement="left" label-width="100" size="small">
                <n-form-item label="名称"><n-input v-model:value="edit.name" placeholder="如 home-wan" /></n-form-item>
                <n-form-item label="服务商"><n-select v-model:value="edit.provider" :options="providerOptions" @update:value="onProvider" /></n-form-item>
                <n-form-item label="启用"><n-switch v-model:value="edit.enabled" /></n-form-item>
                <n-form-item label="域名"><n-input v-model:value="edit.domain" placeholder="example.duckdns.org" /></n-form-item>
                <n-form-item v-if="has('token')" label="Token"><n-input v-model:value="edit.token" type="password" show-password-on="click" :placeholder="edit.tokenSaved ? '已保存，留空保持不变' : 'API Token'" /></n-form-item>
                <n-form-item v-if="has('username')" label="用户名"><n-input v-model:value="edit.username" /></n-form-item>
                <n-form-item v-if="has('password')" label="密码"><n-input v-model:value="edit.password" type="password" show-password-on="click" :placeholder="edit.passwordSaved ? '已保存，留空保持不变' : '密码'" /></n-form-item>
                <n-form-item v-if="has('zone_id')" label="Zone ID"><n-input v-model:value="edit.zone_id" /></n-form-item>
                <n-form-item v-if="has('record_id')" label="Record ID"><n-input v-model:value="edit.record_id" /></n-form-item>
                <n-form-item v-if="has('url')" label="更新 URL"><n-input v-model:value="edit.url" placeholder="https://.../?ip={ip}&domain={domain}" /></n-form-item>
                <n-form-item label="IP 来源"><n-select v-model:value="edit.ip_source" :options="ipSources" /></n-form-item>
                <n-form-item v-if="edit.ip_source === 'interface'" label="接口"><n-input v-model:value="edit.ip_interface" placeholder="eth0 / pppoe0" /></n-form-item>
                <n-form-item v-if="edit.ip_source === 'manual'" label="手动 IP"><n-input v-model:value="edit.manual_ip" /></n-form-item>
                <n-form-item label="间隔（分钟）"><n-input-number v-model:value="edit.interval" :min="1" :max="1440" /></n-form-item>
              </n-form>
              <div class="toolbar" style="margin-bottom: 0">
                <n-button @click="upsert">{{ editIndex >= 0 ? '更新条目' : '添加条目' }}</n-button>
                <n-button v-if="editIndex >= 0" @click="resetEdit">取消编辑</n-button>
                <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
              </div>
              <div class="muted" style="margin-top: 8px">{{ currentNote }}</div>
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="已配置条目" size="small">
              <div class="toolbar">
                <n-switch v-model:value="cfg.enabled" @update:value="save"><template #checked>后台更新已启用</template><template #unchecked>后台更新已停用</template></n-switch>
                <n-button type="warning" @click="apply" :loading="applying">启用并立即更新</n-button>
                <n-button @click="updateNow" :loading="updating">仅立即更新</n-button>
              </div>
              <n-data-table :columns="entryCols" :data="cfg.entries || []" size="small" :bordered="false" />
              <div v-if="!(cfg.entries || []).length" class="muted" style="margin-top: 10px">暂无条目，从左侧添加</div>
            </n-card>
            <n-card title="服务商说明" size="small" style="margin-top: 12px">
              <div v-for="p in providers" :key="p.id" class="mono" style="margin-bottom: 4px">· <strong>{{ p.name }}</strong> — {{ p.note }}</div>
            </n-card>
          </n-grid-item>
        </n-grid>
        <n-alert v-if="resultMsg" :type="resultOk ? 'success' : 'warning'" size="small" style="margin-top: 12px">
          <div v-for="(r, i) in results" :key="i" class="mono">{{ r.ok ? '✓' : '✗' }} {{ r.name }} ({{ r.domain }}) {{ r.ip }} — {{ r.msg }}</div>
          <span v-if="!results.length">{{ resultMsg }}</span>
        </n-alert>
      </n-tab-pane>

      <n-tab-pane name="history" tab="更新历史">
        <div class="toolbar">
          <n-button @click="loadHistory" :loading="loadingHistory">刷新</n-button>
          <n-button type="error" ghost @click="clearHistory">清空历史</n-button>
          <span class="muted">最多保留 {{ historyLimit }} 条</span>
        </div>
        <n-data-table :columns="historyCols" :data="history" size="small" :bordered="false" />
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, computed, h, onMounted } from 'vue'
import { NTag, NButton, NSpace, useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const KEEP = '__BIANWRT_KEEP__'
const message = useMessage()
const cfg = ref({ enabled: false, entries: [] })
const providers = ref([])
const history = ref([])
const historyLimit = ref(60)
const results = ref([])
const resultMsg = ref('')
const resultOk = ref(true)
const editIndex = ref(-1)
const saving = ref(false)
const applying = ref(false)
const updating = ref(false)
const loadingHistory = ref(false)
const ipSources = [
  { label: '外部服务探测公网 IP', value: 'external' },
  { label: '指定接口 IPv4', value: 'interface' },
  { label: '手动指定', value: 'manual' }
]
const blank = () => ({ name: 'ddns1', provider: 'duckdns', enabled: true, domain: '', zone_id: '', record_id: '', token: '', username: '', password: '', ip_source: 'external', ip_interface: '', manual_ip: '', url: '', interval: 10, tokenSaved: false, passwordSaved: false })
const edit = ref(blank())

const providerOptions = computed(() => providers.value.map((p) => ({ label: p.name, value: p.id })))
const currentMeta = computed(() => providers.value.find((p) => p.id === edit.value.provider) || { fields: [] })
const currentNote = computed(() => currentMeta.value.note || '')
function has(f) { return (currentMeta.value.fields || []).includes(f) }
function onProvider() {
  if (edit.value.provider === 'custom' && !edit.value.url) edit.value.url = 'https://example.com/update?ip={ip}&domain={domain}'
}

const entryCols = [
  { title: '名称', key: 'name', width: 110 },
  { title: '服务商', key: 'provider', width: 110 },
  { title: '域名', key: 'domain', width: 190, ellipsis: { tooltip: true } },
  { title: 'IP 来源', key: 'ip_source', width: 120 },
  { title: '间隔', key: 'interval', width: 80, render: (r) => r.interval + ' min' },
  { title: '启用', key: 'enabled', width: 80, render: (r) => h(NTag, { size: 'small', type: r.enabled ? 'success' : 'default' }, { default: () => r.enabled ? '是' : '否' }) },
  { title: '操作', key: 'act', width: 130, render: (r, i) => h(NSpace, { size: 'small' }, { default: () => [
    h(NButton, { size: 'tiny', onClick: () => openEdit(r, i) }, { default: () => '编辑' }),
    h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => delEntry(i) }, { default: () => '删除' })
  ] }) }
]
const historyCols = [
  { title: '时间', key: 'ts', width: 170, render: (r) => new Date(r.ts * 1000).toLocaleString() },
  { title: '条目', key: 'name', width: 110 },
  { title: '域名', key: 'domain', width: 190 },
  { title: '服务商', key: 'provider', width: 110 },
  { title: 'IP', key: 'ip', width: 140 },
  { title: '结果', key: 'ok', width: 80, render: (r) => h(NTag, { size: 'small', type: r.ok ? 'success' : 'error' }, { default: () => r.ok ? '成功' : '失败' }) },
  { title: '详情', key: 'msg', ellipsis: { tooltip: true } }
]

function fromServer(e) {
  return { ...e, token: '', password: '', tokenSaved: e.token === KEEP, passwordSaved: e.password === KEEP }
}
function toServer(e) {
  const { tokenSaved, passwordSaved, ...rest } = e
  return {
    ...rest,
    token: e.token || (tokenSaved ? KEEP : ''),
    password: e.password || (passwordSaved ? KEEP : '')
  }
}
function payload() {
  return { enabled: cfg.value.enabled, entries: (cfg.value.entries || []).map(toServer) }
}

async function loadConfig() {
  try {
    const { data } = await http.get('/ddns/config')
    cfg.value = { enabled: data.config.enabled, entries: (data.config.entries || []).map(fromServer) }
    providers.value = data.providers || []
  } catch (e) { message.error(String(errMsg(e))) }
}

async function loadHistory() {
  loadingHistory.value = true
  try {
    const { data } = await http.get('/ddns/history')
    history.value = data.items || []
    historyLimit.value = data.limit || 60
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingHistory.value = false }
}

function upsert() {
  if (!edit.value.name || !edit.value.domain) { message.warning('请填写名称与域名'); return }
  const item = { ...edit.value }
  const arr = [...(cfg.value.entries || [])]
  const dup = arr.findIndex((x, i) => x.name === item.name && i !== editIndex.value)
  if (dup >= 0) { message.warning('同名条目已存在'); return }
  if (editIndex.value >= 0) arr[editIndex.value] = item
  else arr.push(item)
  cfg.value.entries = arr
  resetEdit()
}
function openEdit(r, i) { edit.value = { ...blank(), ...r }; editIndex.value = i }
function resetEdit() { edit.value = blank(); editIndex.value = -1 }
function delEntry(i) { cfg.value.entries.splice(i, 1); resetEdit() }

async function save() {
  saving.value = true
  try {
    await http.put('/ddns/config', payload())
    message.success('配置已保存')
    await loadConfig()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

function showResults(data) {
  results.value = data.results || []
  resultMsg.value = data.msg || ''
  resultOk.value = results.value.every((r) => r.ok)
}

async function updateNow() {
  updating.value = true
  try {
    await save()
    const { data } = await http.post('/ddns/update')
    showResults(data)
    await loadHistory()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { updating.value = false }
}

async function apply() {
  applying.value = true
  try {
    await save()
    const { data } = await http.post('/ddns/apply')
    showResults(data)
    await loadConfig()
    await loadHistory()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { applying.value = false }
}

async function clearHistory() {
  try { await http.post('/ddns/clear'); message.success('已清空'); await loadHistory() }
  catch (e) { message.error(String(errMsg(e))) }
}

onMounted(() => { loadConfig(); loadHistory() })
</script>
