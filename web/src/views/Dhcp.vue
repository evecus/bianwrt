<template>
  <n-card title="DHCP / DNS (dnsmasq)" style="margin-bottom: 16px">
    <template #header-extra>
      <n-button type="primary" :loading="applying" @click="apply">保存并重启 dnsmasq</n-button>
    </template>
    <n-space vertical size="large">
      <n-switch v-model:value="cfg.enabled">
        <template #checked>已接管</template><template #unchecked>未接管</template>
      </n-switch>
      <n-grid :cols="3" :x-gap="16" :y-gap="8">
        <n-grid-item><n-form-item label="监听接口"><n-input v-model:value="cfg.interface" placeholder="如 eth1，留空=全部" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="域名"><n-input v-model:value="cfg.domain" placeholder="lan" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="缓存条数"><n-input-number v-model:value="cfg.cache_size" :min="0" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="DHCP 起始地址"><n-input v-model:value="cfg.dhcp_start" placeholder="192.168.8.100" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="租约数量"><n-input-number v-model:value="cfg.dhcp_limit" :min="1" :max="4096" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="租约时长(小时)"><n-input-number v-model:value="cfg.lease_hours" :min="1" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="网关"><n-input v-model:value="cfg.gateway" placeholder="192.168.8.1" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="DNS 下发"><n-input v-model:value="dnsJoin" placeholder="逗号分隔，留空=网关" /></n-form-item></n-grid-item>
        <n-grid-item><n-form-item label="上游 DNS">
          <n-select v-model:value="cfg.upstream_dns" multiple filterable tag placeholder="如 223.5.5.5 或 /etc/hosts 文件" :options="[]" />
        </n-form-item></n-grid-item>
      </n-grid>
      <n-checkbox v-model:checked="cfg.no_resolv">不读取 /etc/resolv.conf（仅用上面上游）</n-checkbox>
    </n-space>
  </n-card>

  <n-card title="静态租约" style="margin-bottom: 16px">
    <template #header-extra>
      <n-button size="small" @click="staticModal = true">添加</n-button>
    </template>
    <n-data-table :columns="staticCols" :data="statics" size="small" />
  </n-card>

  <n-card title="活跃租约（在线设备）">
    <template #header-extra>
      <n-button size="small" quaternary @click="loadLeases">刷新</n-button>
    </template>
    <n-data-table :columns="leaseCols" :data="leases" size="small" />
  </n-card>

  <n-modal v-model:show="staticModal" preset="card" style="width: 420px" title="添加静态租约">
    <n-form label-placement="left" label-width="70">
      <n-form-item label="MAC"><n-input v-model:value="newStatic.mac" placeholder="aa:bb:cc:dd:ee:ff" /></n-form-item>
      <n-form-item label="IP"><n-input v-model:value="newStatic.ip" placeholder="192.168.8.10" /></n-form-item>
      <n-form-item label="主机名"><n-input v-model:value="newStatic.hostname" /></n-form-item>
    </n-form>
    <template #footer>
      <n-space justify="end">
        <n-button @click="staticModal = false">取消</n-button>
        <n-button type="primary" @click="addStatic">保存</n-button>
      </n-space>
    </template>
  </n-modal>
</template>

<script setup>
import { ref, onMounted, computed, h } from 'vue'
import {
  NCard, NSwitch, NSpace, NButton, NGrid, NGridItem, NFormItem, NInput, NInputNumber,
  NSelect, NCheckbox, NDataTable, NModal, NForm, NTag, useMessage
} from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const cfg = ref({})
const statics = ref([])
const leases = ref([])
const applying = ref(false)
const staticModal = ref(false)
const newStatic = ref({ mac: '', ip: '', hostname: '' })

const dnsJoin = computed({
  get: () => (cfg.value.dns_servers || []).join(','),
  set: (v) => { cfg.value.dns_servers = v.split(',').map(s => s.trim()).filter(Boolean) }
})

const staticCols = [
  { title: 'MAC', key: 'mac' },
  { title: 'IP', key: 'ip' },
  { title: '主机名', key: 'hostname', render: (r) => r.hostname || '-' },
  {
    title: '操作', key: 'op', width: 90,
    render: (r) => h(NButton, { size: 'tiny', type: 'error', onClick: () => delStatic(r) }, { default: () => '删除' })
  }
]
const leaseCols = [
  { title: 'IP', key: 'ip' },
  { title: 'MAC', key: 'mac' },
  { title: '主机名', key: 'hostname', render: (r) => (r.hostname === '*' ? '-' : r.hostname) },
  {
    title: '租约到期', key: 'expires',
    render: (r) => h(NTag, { size: 'small' }, { default: () => new Date(r.expires * 1000).toLocaleString() })
  }
]

async function load() {
  const { data } = await http.get('/dhcp')
  cfg.value = data.config
  statics.value = data.statics
  loadLeases()
}
async function loadLeases() {
  const { data } = await http.get('/dhcp/leases')
  leases.value = data
}
async function addStatic() {
  try {
    await http.post('/dhcp/static', newStatic.value)
    staticModal.value = false
    newStatic.value = { mac: '', ip: '', hostname: '' }
    load()
    message.success('已添加')
  } catch (e) { message.error(errMsg(e)) }
}
async function delStatic(r) {
  await http.delete(`/dhcp/static/${r.mac}`)
  load()
}
async function apply() {
  applying.value = true
  try {
    await http.put('/dhcp', cfg.value)
    await http.post('/dhcp/apply')
    message.success('已保存并重启 dnsmasq')
  } catch (e) { message.error(errMsg(e)) } finally { applying.value = false }
}
onMounted(load)
</script>
