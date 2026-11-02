<template>
  <n-card title="网络接口（只读）" style="margin-bottom: 16px">
    <template #header-extra>
      <n-tag type="warning" size="small">v1 仅展示，修改请通过 netplan / nmcli</n-tag>
    </template>
    <n-data-table :columns="cols" :data="ifaces" size="small" />
  </n-card>
  <n-card title="路由表">
    <n-tabs type="line">
      <n-tab-pane name="v4" tab="IPv4">
        <n-code :code="pretty(routes.v4)" language="json" />
      </n-tab-pane>
      <n-tab-pane name="v6" tab="IPv6">
        <n-code :code="pretty(routes.v6)" language="json" />
      </n-tab-pane>
    </n-tabs>
  </n-card>
</template>

<script setup>
import { ref, onMounted, h } from 'vue'
import { NCard, NDataTable, NTabs, NTabPane, NTag } from 'naive-ui'
import http from '../lib/http'

const ifaces = ref([])
const routes = ref({})

const cols = [
  { title: '接口', key: 'ifname', width: 140 },
  { title: '状态', key: 'flags', width: 120, render: (r) => h('span', (r.flags || []).join(', ')) },
  {
    title: '地址', key: 'addr',
    render: (r) => h('div', (r.addr_info || []).map(a => `${a.local}/${a.prefixlen}`).join(' · ') || '-')
  },
  { title: 'MAC', key: 'address', width: 200, render: (r) => r.address || '-' },
  { title: 'MTU', key: 'mtu', width: 90, render: (r) => r.mtu ?? '-' }
]

function pretty(v) { return JSON.stringify(v, null, 2) || '无数据' }

onMounted(async () => {
  const [a, r] = await Promise.all([
    http.get('/network/interfaces'),
    http.get('/network/routes')
  ])
  // merge addr + link info by ifname
  const links = {}
  for (const l of a.data.links || []) links[l.ifname] = l
  ifaces.value = (a.data.addrs || []).map(x => ({ ...x, ...(links[x.ifname] || {}) }))
  routes.value = r.data
})
</script>
