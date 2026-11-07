<template>
  <div>
    <div class="toolbar">
      <span class="muted">工具检测：</span>
      <n-tag v-for="(ok, name) in tools" :key="name" size="small" :type="ok ? 'success' : 'default'">
        {{ name }} {{ ok ? '✓' : '✗' }}
      </n-tag>
    </div>
    <n-tabs type="line">
      <n-tab-pane name="ping" tab="Ping">
        <div class="toolbar">
          <n-input v-model:value="host" placeholder="目标 IP 或域名" style="width: 240px" @keyup.enter="runPing" />
          <n-input-number v-model:value="count" :min="1" :max="20" style="width: 120px" />
          <n-select v-model:value="family" :options="families" style="width: 120px" />
          <n-button type="primary" @click="runPing" :loading="busy.ping">执行</n-button>
        </div>
        <pre class="code-block mono" style="min-height: 240px">{{ out.ping || '等待执行…' }}</pre>
      </n-tab-pane>
      <n-tab-pane name="trace" tab="路由追踪">
        <div class="toolbar">
          <n-input v-model:value="host" placeholder="目标 IP 或域名" style="width: 240px" @keyup.enter="runTrace" />
          <n-input-number v-model:value="hops" :min="1" :max="30" style="width: 120px" />
          <n-button type="primary" @click="runTrace" :loading="busy.trace">执行</n-button>
        </div>
        <pre class="code-block mono" style="min-height: 240px">{{ out.trace || '等待执行…' }}</pre>
      </n-tab-pane>
      <n-tab-pane name="dns" tab="DNS 查询">
        <div class="toolbar">
          <n-input v-model:value="dnsHost" placeholder="要解析的域名" style="width: 240px" @keyup.enter="runDns" />
          <n-input v-model:value="dnsServer" placeholder="指定 DNS（可选）" style="width: 200px" clearable />
          <n-button type="primary" @click="runDns" :loading="busy.dns">执行</n-button>
        </div>
        <pre class="code-block mono" style="min-height: 240px">{{ out.dns || '等待执行…' }}</pre>
      </n-tab-pane>
      <n-tab-pane name="tcp" tab="TCP 端口">
        <div class="toolbar">
          <n-input v-model:value="tcpHost" placeholder="目标 IP 或域名" style="width: 240px" @keyup.enter="runTcp" />
          <n-input-number v-model:value="tcpPort" :min="1" :max="65535" style="width: 130px" />
          <n-button type="primary" @click="runTcp" :loading="busy.tcp">探测</n-button>
        </div>
        <pre class="code-block mono" style="min-height: 240px">{{ out.tcp || '等待执行…' }}</pre>
      </n-tab-pane>
      <n-tab-pane name="http" tab="HTTP 探测">
        <div class="toolbar">
          <n-input v-model:value="url" placeholder="https://www.example.com" style="width: 320px" @keyup.enter="runHttp" />
          <n-button type="primary" @click="runHttp" :loading="busy.http">执行</n-button>
        </div>
        <pre class="code-block mono" style="min-height: 240px">{{ out.http || '等待执行…' }}</pre>
      </n-tab-pane>
    </n-tabs>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const tools = ref({})
const host = ref('223.5.5.5')
const count = ref(4)
const family = ref('4')
const hops = ref(15)
const dnsHost = ref('www.baidu.com')
const dnsServer = ref('')
const tcpHost = ref('127.0.0.1')
const tcpPort = ref(8088)
const url = ref('https://www.baidu.com')
const families = [{ label: 'IPv4', value: '4' }, { label: 'IPv6', value: '6' }, { label: '自动', value: 'auto' }]
const busy = ref({ ping: false, trace: false, dns: false, tcp: false, http: false })
const out = ref({ ping: '', trace: '', dns: '', tcp: '', http: '' })

async function run(key, path, payload) {
  busy.value[key] = true
  out.value[key] = '执行中…'
  try {
    const { data } = await http.post(path, payload)
    out.value[key] = `[${data.tool}]\n${data.output || '(无输出)'}`
  } catch (e) {
    out.value[key] = '失败：' + String(errMsg(e))
    message.error(String(errMsg(e)))
  } finally { busy.value[key] = false }
}

const runPing = () => run('ping', '/diag/ping', { host: host.value, count: count.value, family: family.value })
const runTrace = () => run('trace', '/diag/traceroute', { host: host.value, hops: hops.value })
const runDns = () => run('dns', '/diag/nslookup', { host: dnsHost.value, server: dnsServer.value || undefined })
const runTcp = () => run('tcp', '/diag/tcp', { host: tcpHost.value, port: tcpPort.value })
const runHttp = () => run('http', '/diag/http', { url: url.value })

onMounted(async () => {
  try { const { data } = await http.get('/diag/tools'); tools.value = data } catch {}
})
</script>
