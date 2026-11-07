<template>
  <div>
    <n-alert type="info" size="small" style="margin-bottom: 12px" title="托管块机制">
      bianwrt 只管理 <code># BEGIN bianwrt-managed</code> … <code># END bianwrt-managed</code> 之间的任务，
      其余 crontab 条目原样保留、不会修改。语法为标准 crontab（分 时 日 月 周 命令）。
    </n-alert>
    <div class="toolbar">
      <n-select v-model:value="tpl" :options="templates" placeholder="选择常用模板" style="width: 240px" clearable />
      <n-button @click="insertTpl" :disabled="!tpl">插入模板</n-button>
      <n-button @click="managed.push('')">添加一行</n-button>
      <n-button type="primary" @click="save" :loading="saving">保存到 crontab</n-button>
      <n-button @click="load" :loading="loading">重新读取</n-button>
      <span class="muted">{{ available ? 'crontab 可用' : 'crontab 不可用：apt install cron' }}</span>
    </div>
    <n-card title="bianwrt 托管任务" size="small" style="margin-bottom: 12px">
      <div v-for="(l, i) in managed" :key="i" style="display: flex; gap: 8px; margin-bottom: 8px">
        <n-input v-model:value="managed[i]" placeholder="0 3 * * * /usr/sbin/reboot" class="mono" />
        <n-button @click="managed.splice(i, 1)">删除</n-button>
      </div>
      <div v-if="!managed.length" class="muted">暂无托管任务</div>
    </n-card>
    <n-grid :x-gap="12" :cols="2">
      <n-grid-item>
        <n-card title="其他 crontab 条目（只读）" size="small">
          <pre class="code-block mono" style="max-height: 240px">{{ other.join('\n') || '(无)' }}</pre>
        </n-card>
      </n-grid-item>
      <n-grid-item>
        <n-card title="应用后预览" size="small">
          <pre class="code-block mono" style="max-height: 240px">{{ preview }}</pre>
        </n-card>
      </n-grid-item>
    </n-grid>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const managed = ref([])
const other = ref([])
const available = ref(false)
const loading = ref(false)
const saving = ref(false)
const tpl = ref(null)

const templates = [
  { label: '每天 03:00 重启系统', value: '0 3 * * * /usr/sbin/reboot' },
  { label: '每天 04:00 重启 bianwrt', value: '0 4 * * * /usr/bin/systemctl restart bianwrt' },
  { label: '每 5 分钟同步时间', value: '*/5 * * * * /usr/sbin/ntpdate -s ntp.aliyun.com' },
  { label: '每周日 02:00 清理日志', value: '0 2 * * 0 /usr/bin/journalctl --vacuum-time=7d' },
  { label: '每天 01:00 备份配置', value: '0 1 * * * /usr/bin/tar czf /root/bianwrt-backup.tgz /etc/bianwrt' }
]

const preview = computed(() => {
  const head = other.value.filter((l) => l.trim()).join('\n')
  const body = managed.value.filter((l) => l.trim())
  if (!body.length) return head || '(空 crontab)'
  return [head, '# BEGIN bianwrt-managed', ...body.map((l) => l.trim()), '# END bianwrt-managed']
    .filter((x) => x !== '')
    .join('\n')
})

function insertTpl() {
  if (tpl.value) managed.value.push(tpl.value)
  tpl.value = null
}

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/cron')
    managed.value = data.managed || []
    other.value = data.other || []
    available.value = data.available
  } catch (e) { message.error(String(errMsg(e))) }
  finally { loading.value = false }
}

async function save() {
  const lines = managed.value.map((l) => l.trim()).filter((l) => l.length)
  saving.value = true
  try {
    const { data } = await http.put('/cron', { lines })
    message.success('计划任务已写入 crontab')
    await load()
  } catch (e) { message.error(String(errMsg(e))) }
  finally { saving.value = false }
}

load()
</script>
