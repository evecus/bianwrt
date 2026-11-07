<template>
  <div>
    <n-alert v-if="statusMsg" type="warning" size="small" style="margin-bottom: 12px">{{ statusMsg }}</n-alert>
    <n-tabs type="line">
      <n-tab-pane name="status" tab="运行状态">
        <div class="toolbar"><n-button @click="loadStatus" :loading="loadingStatus">刷新</n-button><span class="muted">wg show all dump</span></div>
        <n-card v-for="i in wgIfaces" :key="i.name" size="small" :title="`接口 ${i.name} · 端口 ${i.listen_port ?? '-'}`" style="margin-bottom: 12px">
          <div class="muted mono" style="margin-bottom: 8px">公钥 {{ i.public_key }}</div>
          <n-data-table :columns="peerCols" :data="i.peers || []" size="small" :bordered="false" />
        </n-card>
        <div v-if="!wgIfaces.length" class="muted">暂无运行中的 WireGuard 接口</div>
      </n-tab-pane>

      <n-tab-pane name="config" tab="接口配置">
        <n-grid :x-gap="12" :cols="2">
          <n-grid-item>
            <n-card title="配置文件 /etc/wireguard" size="small">
              <div class="toolbar"><n-button @click="addNew">新建接口</n-button><n-button @click="loadIfaces" :loading="loadingIfaces">刷新</n-button></div>
              <n-data-table :columns="ifaceCols" :data="wgConfigs" size="small" :bordered="false" :loading="loadingIfaces" />
            </n-card>
          </n-grid-item>
          <n-grid-item>
            <n-card :title="`编辑 ${edit.name || '新接口'}.conf`" size="small">
              <n-input v-model:value="edit.name" placeholder="接口名，如 wg0" style="margin-bottom: 8px" :disabled="!edit.isNew" />
              <n-input v-model:value="edit.config" type="textarea" class="mono" :autosize="{ minRows: 10, maxRows: 20 }" />
              <div class="toolbar" style="margin-top: 8px; margin-bottom: 0">
                <n-button type="primary" @click="save" :loading="saving">保存配置</n-button>
                <n-button @click="fillTemplate">填入模板</n-button>
                <n-button v-if="!edit.isNew" @click="action('up')">启用</n-button>
                <n-button v-if="!edit.isNew" @click="action('down')">停用</n-button>
                <n-button v-if="!edit.isNew" @click="action('enable')">开机自启</n-button>
                <n-button v-if="!edit.isNew" @click="action('disable')">取消自启</n-button>
              </div>
              <div class="muted" style="margin-top: 8px">私钥在列表中已脱敏；请到「密钥生成」页生成密钥对后填入编辑器。</div>
            </n-card>
          </n-grid-item>
        </n-grid>
      </n-tab-pane>

      <n-tab-pane name="keys" tab="密钥生成">
        <div class="toolbar">
          <n-button type="primary" @click="keygen" :loading="generating">生成密钥对</n-button>
          <span class="muted">基于 wg genkey / wg pubkey / wg genpsk</span>
        </div>
        <n-grid :x-gap="12" :cols="3">
          <n-grid-item><n-card title="私钥 PrivateKey" size="small"><n-input :value="keys.private" type="textarea" class="mono" :autosize="{ minRows: 2 }" readonly /></n-card></n-grid-item>
          <n-grid-item><n-card title="公钥 PublicKey" size="small"><n-input :value="keys.public" type="textarea" class="mono" :autosize="{ minRows: 2 }" readonly /></n-card></n-grid-item>
          <n-grid-item><n-card title="预共享密钥 PresharedKey" size="small"><n-input :value="keys.preshared" type="textarea" class="mono" :autosize="{ minRows: 2 }" readonly /></n-card></n-grid-item>
        </n-grid>
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, h, onMounted } from 'vue'
import { NButton, NSpace, NTag, useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const statusMsg = ref('')
const wgIfaces = ref([])
const wgConfigs = ref([])
const loadingStatus = ref(false)
const loadingIfaces = ref(false)
const saving = ref(false)
const generating = ref(false)
const edit = ref({ name: 'wg0', config: '', isNew: true })
const keys = ref({ private: '', public: '', preshared: '' })

const peerCols = [
  { title: '对端公钥', key: 'public_key', ellipsis: { tooltip: true } },
  { title: '端点', key: 'endpoint', width: 190 },
  { title: 'AllowedIPs', key: 'allowed_ips', width: 170 },
  { title: '最近握手', key: 'latest_handshake', width: 130, render: (r) => r.latest_handshake ? new Date(r.latest_handshake * 1000).toLocaleString() : '从未' },
  { title: '接收', key: 'rx', width: 100, render: (r) => fmtBytes(r.rx) },
  { title: '发送', key: 'tx', width: 100, render: (r) => fmtBytes(r.tx) }
]

const ifaceCols = [
  { title: '接口', key: 'name', width: 100 },
  { title: '开机自启', key: 'enabled', width: 110, render: (r) => h(NTag, { size: 'small', type: r.enabled ? 'success' : 'default' }, { default: () => r.enabled ? '已启用' : '未启用' }) },
  { title: '大小', key: 'size', width: 90, render: (r) => `${r.size} B` },
  {
    title: '操作', key: 'act', width: 220,
    render: (r) => h(NSpace, { size: 'small' }, {
      default: () => [
        h(NButton, { size: 'tiny', onClick: () => openConfig(r) }, { default: () => '编辑' }),
        h(NButton, { size: 'tiny', onClick: () => action('up', r.name) }, { default: () => '启用' }),
        h(NButton, { size: 'tiny', type: 'error', ghost: true, onClick: () => remove(r) }, { default: () => '删除' })
      ]
    })
  }
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
    const { data } = await http.get('/wg/status')
    wgIfaces.value = data.interfaces || []
    statusMsg.value = data.available ? '' : (data.msg || '')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loadingStatus.value = false }
}

async function loadIfaces() {
  loadingIfaces.value = true
  try { const { data } = await http.get('/wg/interfaces'); wgConfigs.value = data.items || [] }
  catch (e) { message.error(String(errMsg(e))) }
  finally { loadingIfaces.value = false }
}

function addNew() {
  edit.value = { name: 'wg0', config: '', isNew: true }
}

function openConfig(r) {
  edit.value = { name: r.name, config: r.content, isNew: false }
  message.info('列表中私钥已脱敏，如需修改请重新粘贴完整私钥')
}

function fillTemplate() {
  edit.value.config = [
    '[Interface]',
    `PrivateKey = ${keys.value.private || '<服务端私钥>'}`,
    'Address = 10.0.0.1/24',
    'ListenPort = 51820',
    '',
    '[Peer]',
    `PublicKey = ${keys.value.public || '<对端公钥>'}`,
    'AllowedIPs = 10.0.0.2/32',
    keys.value.preshared ? `PresharedKey = ${keys.value.preshared}` : '# PresharedKey = <可选>'
  ].join('\n')
}

async function save() {
  if (!edit.value.name || !edit.value.config.includes('[Interface]')) { message.warning('请填写接口名与 [Interface] 配置'); return }
  saving.value = true
  try {
    await http.put('/wg/interface', { name: edit.value.name, config: edit.value.config })
    message.success('配置已保存（权限 600）')
    edit.value.isNew = false
    await loadIfaces()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

async function action(op, name) {
  const n = name || edit.value.name
  if (!n) return
  try {
    const { data } = await http.post('/wg/action', { name: n, op })
    message.success(data.msg || '操作成功')
    await Promise.all([loadIfaces(), loadStatus()])
  } catch (e) { message.error(String(errMsg(e))) }
}

function remove(r) {
  dialog.warning({
    title: '删除配置', content: `确定删除 /etc/wireguard/${r.name}.conf？`,
    positiveText: '删除', negativeText: '取消',
    onPositiveClick: async () => {
      try { await http.delete(`/wg/interface/${encodeURIComponent(r.name)}`); message.success('已删除'); await loadIfaces() }
      catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

async function keygen() {
  generating.value = true
  try {
    const { data } = await http.post('/wg/keygen')
    keys.value = { private: data.private, public: data.public, preshared: data.preshared }
    message.success('密钥对已生成')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { generating.value = false }
}

onMounted(() => { loadStatus(); loadIfaces() })
</script>
