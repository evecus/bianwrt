<template>
  <div>
    <n-alert type="warning" size="small" style="margin-bottom: 12px" title="网络变更提示">
      「即时生效」的操作直接调用 ip 命令，重启后丢失；持久化配置写入 /etc/network/interfaces.d，
      需显式点击「应用网络配置」才会重启网络服务，可能短暂中断 SSH 连接，请确认有带外访问方式。
    </n-alert>
    <n-tabs type="line">
      <!-- 接口 -->
      <n-tab-pane name="ifaces" tab="接口管理">
        <div class="toolbar">
          <n-button @click="loadIfaces" :loading="loadingIfaces">刷新</n-button>
          <n-select v-model:value="addrForm.name" :options="ifaceOptions" placeholder="选择接口" style="width: 160px" />
          <n-input v-model:value="addrForm.addr" placeholder="192.168.1.1/24" style="width: 200px" />
          <n-button @click="addrOp('add')">添加地址</n-button>
          <n-button @click="addrOp('del')" type="warning" ghost>删除地址</n-button>
        </div>
        <n-data-table :columns="ifaceCols" :data="ifaces" size="small" :bordered="false" :loading="loadingIfaces" />
      </n-tab-pane>
      <!-- 静态路由 -->
      <n-tab-pane name="routes" tab="静态路由">
        <div class="toolbar">
          <n-input v-model:value="form.dest" placeholder="目标网段，如 10.0.0.0/8 或 default" style="width: 240px" />
          <n-select v-model:value="form.family" :options="families" style="width: 100px" />
          <n-input v-model:value="form.via" placeholder="下一跳 via" style="width: 160px" />
          <n-input v-model:value="form.dev" placeholder="出接口 dev" style="width: 140px" />
          <n-input v-model:value="form.metric" placeholder="metric" style="width: 100px" />
          <n-input v-model:value="form.table" placeholder="table" style="width: 100px" />
          <n-button type="primary" @click="addRoute">添加路由</n-button>
          <n-button @click="loadRoutes" :loading="loadingRoutes">刷新</n-button>
        </div>
        <n-tabs type="segment" size="small">
          <n-tab-pane name="v4" tab="IPv4">
            <n-data-table :columns="routeCols('4')" :data="routes.v4" size="small" :bordered="false" :loading="loadingRoutes" />
          </n-tab-pane>
          <n-tab-pane name="v6" tab="IPv6">
            <n-data-table :columns="routeCols('6')" :data="routes.v6" size="small" :bordered="false" :loading="loadingRoutes" />
          </n-tab-pane>
        </n-tabs>
      </n-tab-pane>
      <!-- 持久化 -->
      <n-tab-pane name="persist" tab="持久化接口配置">
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="生成 ifupdown 配置" size="small">
              <n-form label-placement="left" label-width="90" size="small">
                <n-form-item label="接口名"><n-input v-model:value="gen.name" placeholder="lan0 / eth0" /></n-form-item>
                <n-form-item label="模式">
                  <n-select v-model:value="gen.mode" :options="[{label:'DHCP',value:'dhcp'},{label:'静态地址',value:'static'},{label:'不配置IP',value:'manual'}]" />
                </n-form-item>
                <n-form-item v-if="gen.mode === 'static'" label="地址"><n-input v-model:value="gen.address" placeholder="192.168.1.1/24" /></n-form-item>
                <n-form-item v-if="gen.mode === 'static'" label="网关"><n-input v-model:value="gen.gateway" placeholder="192.168.1.254" /></n-form-item>
                <n-form-item v-if="gen.mode === 'static'" label="DNS"><n-input v-model:value="gen.dns" placeholder="223.5.5.5 1.1.1.1" /></n-form-item>
                <n-form-item label="MTU"><n-input-number v-model:value="gen.mtu" :min="576" :max="9000" clearable /></n-form-item>
              </n-form>
              <n-button @click="generate" :loading="generating">生成配置</n-button>
              <n-input v-model:value="persistContent" type="textarea" class="mono" :autosize="{ minRows: 7, maxRows: 14 }" style="margin-top: 10px" />
              <div class="toolbar" style="margin-top: 8px; margin-bottom: 0">
                <n-button type="primary" @click="savePersist" :loading="saving">保存为文件</n-button>
                <n-button type="error" @click="applyPersist">应用网络配置</n-button>
                <span class="muted">{{ persistInfo }}</span>
              </div>
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card title="已保存的配置文件" size="small">
              <n-data-table :columns="persistCols" :data="persistItems" size="small" :bordered="false" />
              <div v-if="!persistItems.length" class="muted" style="margin-top: 10px">暂无 bianwrt 管理的接口配置</div>
            </n-card>
          </n-grid-item>
        </n-grid>
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
const ifaces = ref([])
const routes = ref({ v4: [], v6: [] })
const loadingIfaces = ref(false)
const loadingRoutes = ref(false)
const generating = ref(false)
const saving = ref(false)
const persistItems = ref([])
const persistContent = ref('')
const persistInfo = ref('')
const addrForm = ref({ name: null, addr: '' })
const form = ref({ dest: '', family: '4', via: '', dev: '', metric: '', table: '' })
const gen = ref({ name: 'lan0', mode: 'dhcp', address: '', gateway: '', dns: '', mtu: null })
const families = [{ label: 'IPv4', value: '4' }, { label: 'IPv6', value: '6' }]

const ifaceOptions = computed(() => ifaces.value.map((i) => ({ label: i.name, value: i.name })))

const ifaceCols = [
  { title: '接口', key: 'name', width: 140 },
  {
    title: '状态', key: 'up', width: 90,
    render: (r) => h(NTag, { size: 'small', type: r.up ? 'success' : 'default' }, { default: () => r.up ? 'UP' : 'DOWN' })
  },
  { title: 'MAC', key: 'mac', width: 180 },
  { title: 'MTU', key: 'mtu', width: 80 },
  { title: '链路状态', key: 'state', width: 110 },
  {
    title: 'IP 地址', key: 'addrs',
    render: (r) => (r.addrs || []).map((a) => `${a.local}/${a.prefixlen}`).join(' · ') || '-'
  },
  {
    title: '操作', key: 'act', width: 110,
    render: (r) => h(NButton, {
      size: 'tiny', disabled: r.name === 'lo',
      onClick: () => toggleLink(r)
    }, { default: () => r.up ? '停用' : '启用' })
  }
]

function routeCols(fam) {
  return [
    { title: '目标', key: 'dst', width: 180, render: (r) => r.dst ?? '-' },
    { title: '下一跳', key: 'via', width: 160, render: (r) => r.via ?? '-' },
    { title: '出接口', key: 'dev', width: 120, render: (r) => r.dev ?? '-' },
    { title: '协议', key: 'protocol', width: 90, render: (r) => r.protocol ?? '-' },
    { title: 'Metric', key: 'metric', width: 90, render: (r) => r.metric ?? '-' },
    { title: '路由表', key: 'table', width: 100, render: (r) => r.table ?? '-' },
    {
      title: '操作', key: 'act', width: 100,
      render: (r) => h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => delRoute(r, fam) }, { default: () => '删除' })
    }
  ]
}

const persistCols = [
  { title: '配置名', key: 'name', width: 130 },
  { title: '文件', key: 'file', width: 200 },
  {
    title: '操作', key: 'act', width: 160,
    render: (r) => h(NSpace, { size: 'small' }, {
      default: () => [
        h(NButton, { size: 'tiny', onClick: () => { persistContent.value = r.content; gen.value.name = r.name } }, { default: () => '载入' }),
        h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => delPersist(r) }, { default: () => '删除' })
      ]
    })
  }
]

async function loadIfaces() {
  loadingIfaces.value = true
  try { const { data } = await http.get('/netcfg/interfaces'); ifaces.value = data.items || [] }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingIfaces.value = false }
}

async function loadRoutes() {
  loadingRoutes.value = true
  try { const { data } = await http.get('/netcfg/routes'); routes.value = { v4: data.v4 || [], v6: data.v6 || [] } }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingRoutes.value = false }
}

async function loadPersist() {
  try {
    const { data } = await http.get('/netcfg/persist')
    persistItems.value = data.items || []
    persistInfo.value = data.netplan ? '检测到 netplan，仍使用 ifupdown 兼容目录' : (data.exists ? data.dir : '目录不存在，保存时自动创建')
  } catch (e) { message.error(String(errMsg(e))) }
}

async function toggleLink(r) {
  try {
    await http.post('/netcfg/interfaces/link', { name: r.name, up: !r.up })
    message.success(`${r.name} 已${r.up ? '停用' : '启用'}`)
    await loadIfaces()
  } catch (e) { message.error(String(errMsg(e))) }
}

async function addrOp(op) {
  if (!addrForm.value.name || !addrForm.value.addr) { message.warning('请选择接口并填写地址'); return }
  try {
    await http.post('/netcfg/interfaces/addr', { name: addrForm.value.name, op, addr: addrForm.value.addr })
    message.success('操作成功（即时生效）'); await loadIfaces()
  } catch (e) { message.error(String(errMsg(e))) }
}

function routePayload() {
  const p = {
    dest: form.value.dest.trim(),
    family: form.value.family
  }
  if (form.value.via.trim()) p.via = form.value.via.trim()
  if (form.value.dev.trim()) p.dev = form.value.dev.trim()
  if (form.value.metric.trim()) p.metric = Number(form.value.metric)
  if (form.value.table.trim()) p.table = form.value.table.trim()
  return p
}

async function addRoute() {
  if (!form.value.dest.trim()) { message.warning('请填写目标网段'); return }
  try {
    const { data } = await http.post('/netcfg/routes', routePayload())
    message.success(data.cmd); await loadRoutes()
  } catch (e) { message.error(String(errMsg(e))) }
}

function delRoute(r, fam) {
  const p = { dest: r.dst || 'default', family: fam }
  if (r.via) p.via = r.via
  if (r.dev) p.dev = r.dev
  if (r.metric != null) p.metric = Number(r.metric)
  if (r.table) p.table = String(r.table)
  dialog.warning({
    title: '删除路由', content: `确定删除到 ${p.dest} 的路由？此操作即时生效。`,
    positiveText: '删除', negativeText: '取消',
    onPositiveClick: async () => {
      try { await http.delete('/netcfg/routes', { data: p }); message.success('已删除'); await loadRoutes() }
      catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

async function generate() {
  generating.value = true
  try {
    const { data } = await http.post('/netcfg/persist/generate', {
      name: gen.value.name, mode: gen.value.mode,
      address: gen.value.address || undefined,
      gateway: gen.value.gateway || undefined,
      dns: gen.value.dns || undefined,
      mtu: gen.value.mtu || undefined
    })
    persistContent.value = data.config
  } catch (e) { message.error(String(errMsg(e))) }
  finally { generating.value = false }
}

async function savePersist() {
  if (!persistContent.value.trim()) { message.warning('请先生成或填写配置'); return }
  saving.value = true
  try {
    await http.put('/netcfg/persist', { name: gen.value.name, content: persistContent.value })
    message.success('配置文件已保存（尚未应用）'); await loadPersist()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

async function delPersist(r) {
  try { await http.delete(`/netcfg/persist/file/${encodeURIComponent(r.name)}`); message.success('已删除'); await loadPersist() }
  catch (e) { message.error(String(errMsg(e))) }
}

function applyPersist() {
  dialog.warning({
    title: '应用网络配置',
    content: '将执行 ifreload -a 或重启 networking 服务，网络会短暂中断。如果通过 SSH 连接，请确认配置正确、有带外恢复方式。',
    positiveText: '我已确认，执行', negativeText: '取消',
    onPositiveClick: async () => {
      try { const { data } = await http.post('/netcfg/persist/apply', { confirm: true }); message.success(`已通过 ${data.tool} 应用`) }
      catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

onMounted(() => { loadIfaces(); loadRoutes(); loadPersist() })
</script>
