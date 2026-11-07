<template>
  <div>
    <n-alert v-if="!info.root" type="warning" size="small" style="margin-bottom: 12px" title="权限提示">
      当前服务并非以 root 运行，主机名 / 时区 / NTP / 重启等设置可能失败。建议以 systemd 服务方式运行。
    </n-alert>
    <div class="toolbar">
      <n-button @click="load" :loading="loading">刷新</n-button>
      <span class="muted">主机名 {{ info.hostname }} · 本地时间 {{ info.localtime }} · NTP {{ info.ntp }} ({{ info.ntp_synchronized }})</span>
    </div>
    <n-grid :x-gap="12" :y-gap="12" :cols="2" responsive="screen" :collapsed="collapsed" :collapsed-rows="1">
      <n-grid-item>
        <n-card title="主机与时间" size="small">
          <div class="section-title">主机名</div>
          <n-input-group>
            <n-input v-model:value="hostname" placeholder="如 openwrt-box" />
            <n-button @click="saveHostname" :loading="savingHost">保存</n-button>
          </n-input-group>
          <div class="section-title">时区</div>
          <n-input-group>
            <n-select v-model:value="timezone" filterable tag :options="tzOptions" placeholder="Asia/Shanghai" style="width: 100%" />
            <n-button @click="saveTimezone" :loading="savingTz">保存</n-button>
          </n-input-group>
          <div class="section-title">网络时间同步 (NTP)</div>
          <n-space align="center">
            <n-switch v-model:value="ntp" @update:value="setNtp" />
            <span class="muted">{{ ntp ? '已启用' : '已停用' }} timedatectl set-ntp</span>
          </n-space>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="修改管理员密码" size="small">
          <n-form :model="pwd" label-placement="left" label-width="90">
            <n-form-item label="原密码"><n-input v-model:value="pwd.old" type="password" show-password-on="click" /></n-form-item>
            <n-form-item label="新密码"><n-input v-model:value="pwd.new" type="password" show-password-on="click" /></n-form-item>
          </n-form>
          <n-button @click="savePwd" :loading="savingPwd">更新密码</n-button>
          <div class="muted" style="margin-top: 8px">更新后当前会话仍有效。</div>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="SSH 公钥 (authorized_keys)" size="small">
          <n-input v-model:value="keysText" type="textarea" class="mono" :autosize="{ minRows: 5, maxRows: 12 }" placeholder="ssh-ed25519 AAAA... user@host" />
          <div class="toolbar" style="margin-top: 8px; margin-bottom: 0">
            <n-button @click="saveKeys" :loading="savingKeys">保存公钥</n-button>
            <span class="muted">{{ info.ssh_keys_path }}</span>
          </div>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="电源操作" size="small">
          <n-space>
            <n-button type="warning" @click="power('reboot')">重启系统</n-button>
            <n-button type="error" @click="power('poweroff')">关机</n-button>
          </n-space>
          <div class="muted" style="margin-top: 10px">操作会在 0.8 秒后执行，页面将断开连接。</div>
        </n-card>
      </n-grid-item>
    </n-grid>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useMessage, useDialog } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const dialog = useDialog()
const info = ref({})
const hostname = ref('')
const timezone = ref('')
const tzOptions = ref([])
const ntp = ref(false)
const keysText = ref('')
const pwd = ref({ old: '', new: '' })
const loading = ref(false)
const savingHost = ref(false)
const savingTz = ref(false)
const savingPwd = ref(false)
const savingKeys = ref(false)
const collapsed = ref(false)

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/system/info')
    info.value = data
    hostname.value = data.hostname || ''
    timezone.value = data.timezone || ''
    ntp.value = data.ntp === 'yes'
    keysText.value = (data.ssh_keys || []).join('\n')
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function call(fn, flag) {
  flag.value = true
  try { await fn() } catch (e) { message.error(String(errMsg(e))) }
  finally { flag.value = false }
}

const saveHostname = () => call(async () => {
  await http.post('/system/hostname', { hostname: hostname.value })
  message.success('主机名已更新'); await load()
}, savingHost)

const saveTimezone = () => call(async () => {
  await http.post('/system/timezone', { timezone: timezone.value })
  message.success('时区已更新'); await load()
}, savingTz)

async function setNtp(v) {
  try { await http.post('/system/ntp', { enabled: v }); message.success(v ? 'NTP 已启用' : 'NTP 已停用') }
  catch (e) { message.error(String(errMsg(e))); ntp.value = !v }
}

const savePwd = () => call(async () => {
  await http.post('/system/password', { old: pwd.value.old, new: pwd.value.new })
  message.success('密码已更新'); pwd.value = { old: '', new: '' }
}, savingPwd)

const saveKeys = () => call(async () => {
  const lines = keysText.value.split('\n').map((s) => s.trim()).filter((s) => s.length)
  await http.put('/system/sshkeys', { lines })
  message.success('SSH 公钥已写入'); await load()
}, savingKeys)

function power(op) {
  dialog.warning({
    title: op === 'reboot' ? '确认重启' : '确认关机',
    content: op === 'reboot' ? '系统即将重启，所有连接会中断。' : '系统即将关机，需要物理开机才能恢复。',
    positiveText: '执行', negativeText: '取消',
    onPositiveClick: async () => {
      try { const { data } = await http.post('/system/power', { op }); message.success(data.msg) }
      catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

onMounted(async () => {
  await load()
  try {
    const { data } = await http.get('/system/timezones')
    tzOptions.value = (data.items || []).map((v) => ({ label: v, value: v }))
  } catch { /* 非 systemd 系统忽略 */ }
})
</script>
