<template>
  <n-card title="防火墙 (nftables)" style="margin-bottom: 16px">
    <template #header-extra>
      <n-space>
        <n-button @click="previewVisible = true">预览规则</n-button>
        <n-button type="primary" :loading="applying" @click="apply">应用规则</n-button>
      </n-space>
    </template>
    <n-space vertical size="large">
      <n-space>
        <n-switch v-model:value="cfg.enabled">
          <template #checked>已启用</template><template #unchecked>未启用</template>
        </n-switch>
        <n-switch v-model:value="cfg.masquerade">
          <template #checked>NAT (masquerade) 开</template><template #unchecked>NAT 关</template>
        </n-switch>
        <n-switch v-model:value="cfg.allow_ping">
          <template #checked>允许 WAN Ping</template><template #unchecked>拒绝 WAN Ping</template>
        </n-switch>
      </n-space>
      <n-alert v-if="!cfg.enabled" type="info" :bordered="false">
        防火墙未启用时面板不接管流量。启用后：LAN 全放行，WAN 仅放行下方规则并丢弃其余入站。首次应用会自动加一条放行 SSH(22) 的保底规则。
      </n-alert>

      <n-h6 style="margin: 0">接口分区</n-h6>
      <n-grid :cols="2" :x-gap="16">
        <n-grid-item>
          <n-card size="small" title="WAN（公网/上游）">
            <n-select v-model:value="wanIfaces" multiple filterable tag placeholder="选择或输入接口名" :options="ifaceOptions" />
          </n-card>
        </n-grid-item>
        <n-grid-item>
          <n-card size="small" title="LAN（内网）">
            <n-select v-model:value="lanIfaces" multiple filterable tag placeholder="选择或输入接口名" :options="ifaceOptions" />
          </n-card>
        </n-grid-item>
      </n-grid>
    </n-space>
  </n-card>

  <n-card title="入站放行规则（作用于 WAN）" style="margin-bottom: 16px">
    <template #header-extra>
      <n-button size="small" @click="editRule(null)">添加规则</n-button>
    </template>
    <n-data-table :columns="ruleCols" :data="cfg.rules" size="small" />
  </n-card>

  <n-card title="端口转发 (DNAT)">
    <template #header-extra>
      <n-button size="small" @click="editFwd(null)">添加转发</n-button>
    </template>
    <n-data-table :columns="fwdCols" :data="cfg.forwards" size="small" />
  </n-card>

  <n-modal v-model:show="ruleModal" preset="card" style="width: 460px" :title="editRuleForm.id ? '编辑规则' : '添加规则'">
    <n-form label-placement="left" label-width="80">
      <n-form-item label="协议">
        <n-select v-model:value="editRuleForm.proto" :options="[{label:'TCP',value:'tcp'},{label:'UDP',value:'udp'},{label:'TCP+UDP',value:'all'},{label:'ICMP',value:'icmp'}]" />
      </n-form-item>
      <n-form-item label="端口">
        <n-input v-model:value="editRuleForm.port" placeholder="如 22 或 80,443 或 1000-2000" />
      </n-form-item>
      <n-form-item label="来源限制">
        <n-input v-model:value="editRuleForm.src" placeholder="留空=任意，如 192.168.1.0/24" />
      </n-form-item>
      <n-form-item label="动作">
        <n-select v-model:value="editRuleForm.action" :options="[{label:'放行',value:'accept'},{label:'拒绝',value:'drop'}]" />
      </n-form-item>
      <n-form-item label="备注">
        <n-input v-model:value="editRuleForm.comment" />
      </n-form-item>
    </n-form>
    <template #footer>
      <n-space justify="end">
        <n-button @click="ruleModal = false">取消</n-button>
        <n-button type="primary" @click="saveRule">保存</n-button>
      </n-space>
    </template>
  </n-modal>

  <n-modal v-model:show="fwdModal" preset="card" style="width: 460px" :title="editFwdForm.id ? '编辑转发' : '添加转发'">
    <n-form label-placement="left" label-width="100">
      <n-form-item label="协议">
        <n-select v-model:value="editFwdForm.proto" :options="[{label:'TCP',value:'tcp'},{label:'UDP',value:'udp'}]" />
      </n-form-item>
      <n-form-item label="外部端口">
        <n-input v-model:value="editFwdForm.src_port" placeholder="如 8080" />
      </n-form-item>
      <n-form-item label="内部目标 IP">
        <n-input v-model:value="editFwdForm.dest" placeholder="如 192.168.1.100" />
      </n-form-item>
      <n-form-item label="内部端口">
        <n-input v-model:value="editFwdForm.dest_port" placeholder="留空=同外部端口" />
      </n-form-item>
      <n-form-item label="备注">
        <n-input v-model:value="editFwdForm.comment" />
      </n-form-item>
    </n-form>
    <template #footer>
      <n-space justify="end">
        <n-button @click="fwdModal = false">取消</n-button>
        <n-button type="primary" @click="saveFwd">保存</n-button>
      </n-space>
    </template>
  </n-modal>

  <n-modal v-model:show="previewVisible" preset="card" style="width: 700px" title="生成的 nftables 规则">
    <n-code :code="lastScript || '点击「应用规则」后生成'" language="bash" />
  </n-modal>
</template>

<script setup>
import { ref, onMounted, computed, h } from 'vue'
import {
  NCard, NSwitch, NSpace, NButton, NDataTable, NModal, NForm, NFormItem,
  NSelect, NInput, NGrid, NGridItem, NAlert, NH6, NTag, NCode, useMessage
} from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const cfg = ref({ enabled: false, masquerade: true, allow_ping: true, zones: [], rules: [], forwards: [] })
const ifaceOptions = ref([])
const applying = ref(false)
const previewVisible = ref(false)
const lastScript = ref('')
const ruleModal = ref(false)
const fwdModal = ref(false)
const editRuleForm = ref({})
const editFwdForm = ref({})

const wanIfaces = computed({
  get: () => cfg.value.zones.find(z => z.name === 'wan')?.ifaces || [],
  set: (v) => {
    let z = cfg.value.zones.find(z => z.name === 'wan')
    if (z) z.ifaces = v; else cfg.value.zones.push({ name: 'wan', ifaces: v })
  }
})
const lanIfaces = computed({
  get: () => cfg.value.zones.find(z => z.name === 'lan')?.ifaces || [],
  set: (v) => {
    let z = cfg.value.zones.find(z => z.name === 'lan')
    if (z) z.ifaces = v; else cfg.value.zones.push({ name: 'lan', ifaces: v })
  }
})

const enabledTag = (r) => h(NTag, { type: r.enabled ? 'success' : 'default', size: 'small' }, { default: () => (r.enabled ? '启用' : '停用') })

const ruleCols = [
  { title: '状态', key: 'enabled', width: 70, render: enabledTag },
  { title: '协议', key: 'proto', width: 90 },
  { title: '端口', key: 'port', width: 140 },
  { title: '来源', key: 'src', render: (r) => r.src || '任意' },
  { title: '动作', key: 'action', width: 90, render: (r) => h(NTag, { type: r.action === 'accept' ? 'success' : 'error', size: 'small' }, { default: () => (r.action === 'accept' ? '放行' : '拒绝') }) },
  { title: '备注', key: 'comment' },
  {
    title: '操作', key: 'op', width: 130,
    render: (r) => h('div', { style: 'display:flex;gap:6px' }, [
      h(NButton, { size: 'tiny', onClick: () => editRule(r) }, { default: () => '编辑' }),
      h(NButton, { size: 'tiny', type: 'error', onClick: () => delRule(r) }, { default: () => '删除' })
    ])
  }
]

const fwdCols = [
  { title: '状态', key: 'enabled', width: 70, render: enabledTag },
  { title: '协议', key: 'proto', width: 90 },
  { title: '外部端口', key: 'src_port', width: 110 },
  { title: '目标', key: 'dest', render: (r) => r.dest_port && r.dest_port !== r.src_port ? `${r.dest}:${r.dest_port}` : r.dest },
  { title: '备注', key: 'comment' },
  {
    title: '操作', key: 'op', width: 130,
    render: (r) => h('div', { style: 'display:flex;gap:6px' }, [
      h(NButton, { size: 'tiny', onClick: () => editFwd(r) }, { default: () => '编辑' }),
      h(NButton, { size: 'tiny', type: 'error', onClick: () => delFwd(r) }, { default: () => '删除' })
    ])
  }
]

function editRule(r) {
  editRuleForm.value = r ? { ...r } : { id: 0, zone: 'wan', proto: 'tcp', port: '', src: '', action: 'accept', comment: '', enabled: true }
  ruleModal.value = true
}
function saveRule() {
  const f = editRuleForm.value
  if (!f.port.trim() && f.proto !== 'icmp') return message.error('请填写端口')
  if (f.id) {
    const i = cfg.value.rules.findIndex(x => x.id === f.id)
    if (i >= 0) cfg.value.rules[i] = { ...f }
  } else {
    f.id = Date.now()
    cfg.value.rules.push({ ...f })
  }
  ruleModal.value = false
}
function delRule(r) {
  cfg.value.rules = cfg.value.rules.filter(x => x.id !== r.id)
}
function editFwd(r) {
  editFwdForm.value = r ? { ...r } : { id: 0, proto: 'tcp', src_port: '', dest: '', dest_port: '', comment: '', enabled: true }
  fwdModal.value = true
}
function saveFwd() {
  const f = editFwdForm.value
  if (!f.src_port.trim() || !f.dest.trim()) return message.error('请填写外部端口与目标 IP')
  if (f.id) {
    const i = cfg.value.forwards.findIndex(x => x.id === f.id)
    if (i >= 0) cfg.value.forwards[i] = { ...f }
  } else {
    f.id = Date.now()
    cfg.value.forwards.push({ ...f })
  }
  fwdModal.value = false
}
function delFwd(r) {
  cfg.value.forwards = cfg.value.forwards.filter(x => x.id !== r.id)
}

async function load() {
  const [{ data: c }, { data: n }] = await Promise.all([
    http.get('/firewall'), http.get('/network/interfaces')
  ])
  cfg.value = c
  const names = new Set()
  for (const i of n.addrs || []) names.add(i.ifname)
  ifaceOptions.value = [...names].map(x => ({ label: x, value: x }))
}

async function save() {
  await http.put('/firewall', cfg.value)
}
async function apply() {
  applying.value = true
  try {
    await save()
    const { data } = await http.post('/firewall/apply')
    lastScript.value = data.script || ''
    message.success('防火墙规则已应用')
  } catch (e) {
    message.error(errMsg(e))
  } finally { applying.value = false }
}

onMounted(load)
</script>
