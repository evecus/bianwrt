<template>
  <n-card title="插件中心" style="margin-bottom: 16px">
    <template #header-extra>
      <n-space>
        <n-button size="small" @click="showDoc = true">插件规范</n-button>
        <n-button type="primary" size="small" @click="triggerUpload">安装 .rpkg 插件包</n-button>
        <input ref="fileInput" type="file" accept=".rpkg,.tgz,.tar.gz" style="display: none" @change="onFile" />
      </n-space>
    </template>
    <n-spin :show="loading">
      <n-alert v-if="hasWebUi" type="info" :bordered="false" style="margin-bottom: 12px">
        带界面的插件已出现在左侧栏「服务」分组，点击即可进入插件页面；这里只负责安装与管理。
      </n-alert>
      <n-grid :x-gap="16" :y-gap="16" :cols="3" v-if="plugins.length">
        <n-grid-item v-for="p in plugins" :key="p.name">
          <n-card size="small">
            <template #header>
              {{ p.name }} <n-tag size="small" style="margin-left: 6px">v{{ p.version }}</n-tag>
            </template>
            <div style="min-height: 42px; color: #aaa; font-size: 13px">{{ p.description || '（无描述）' }}</div>
            <n-space style="margin-top: 8px" size="small">
              <n-tag v-if="p.has_service" :type="p.active ? 'success' : p.service_exists ? 'warning' : 'default'" size="small">
                {{ p.active ? '运行中' : p.service_exists ? '已停止' : '服务未注册' }}
              </n-tag>
              <n-tag v-else size="small" type="info">纯页面插件</n-tag>
            </n-space>
            <template #footer>
              <n-space size="small">
                <template v-if="p.has_service">
                  <n-button v-if="!p.active" size="tiny" type="primary" @click="act(p, 'start')">启动</n-button>
                  <n-button v-else size="tiny" @click="act(p, 'stop')">停止</n-button>
                  <n-button size="tiny" @click="act(p, 'restart')">重启</n-button>
                </template>
                <n-button size="tiny" type="error" @click="askUninstall(p)">卸载</n-button>
              </n-space>
            </template>
          </n-card>
        </n-grid-item>
      </n-grid>
      <n-empty v-else description="尚未安装任何插件，点击右上角上传 .rpkg 包安装" style="padding: 60px 0" />
    </n-spin>
  </n-card>

  <n-modal v-model:show="uninstallVisible" preset="dialog" type="warning" :title="`卸载 ${uninstallTarget?.name}`" :positive-text="'确认卸载'" :negative-text="'取消'" :loading="uninstalling" @positive-click="confirmUninstall">
    <n-space vertical>
      <div>将按文件清单精确清理插件铺到系统的文件，并删除插件目录与 systemd 服务。</div>
      <n-checkbox v-model:checked="purgeConfig">同时删除配置目录 /etc/bianwrt/plugins/{{ uninstallTarget?.name }}（含用户配置，默认保留）</n-checkbox>
    </n-space>
  </n-modal>

  <n-modal v-model:show="showDoc" preset="card" style="width: 720px" title=".rpkg 插件包规范">
    <n-code :code="docText" language="markdown" />
  </n-modal>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import {
  NCard, NGrid, NGridItem, NButton, NTag, NSpace, NSpin, NEmpty, NModal, NAlert,
  NCheckbox, NCode, useMessage
} from 'naive-ui'
import http, { errMsg } from '../lib/http'

const message = useMessage()
const plugins = ref([])
const loading = ref(false)
const fileInput = ref(null)
const showDoc = ref(false)

const hasWebUi = computed(() => plugins.value.some((p) => p.web_mode === 'static'))

const docText = `# .rpkg 插件包规范

.rpkg = tar.gz 压缩包，结构：

  manifest.toml        # 必需：插件元信息
  rootfs/              # 可选：完整根目录结构，安装时直接铺到 /
    usr/bin/xxx        #   如 /usr/bin/xxx
    etc/my-plugin/     #   系统侧配置文件
  ui/                  # 可选：前端静态页面 (SPA, 含 index.html)

manifest.toml 示例：

  [package]
  name = "my-plugin"        # 小写字母/数字/-
  version = "1.0.0"
  description = "示例插件"
  author = "you"
  web_mode = "static"
  conffiles = ["etc/my-plugin/conf.json"]   # 可选：升级时保留用户版本

  [service]                  # 可选：注册 systemd 服务
  exec = "bin/my-plugin"     # 相对包根目录
  args = ["--port", "9090"]
  user = "root"

  [deps]                     # 可选：apt 依赖，安装时校验
  apt = ["curl", "dnsmasq"]

安装后：
- rootfs/ 铺到真实根目录（/usr/bin、/etc/... 全根结构，同 OpenWrt ipk）
- 文件清单存于 /var/lib/bianwrt/plugin-files/<name>.list（卸载精确清理的依据）
- 插件配置目录 /etc/bianwrt/plugins/<name>/（同 /etc/config，卸载默认保留）
- ui 挂载于 /plugins/<name>/，带界面的插件出现在左侧栏「服务」分组
- systemd 服务名：bianwrt-plugin-<name>.service`

function triggerUpload() {
  fileInput.value.click()
}
async function onFile(e) {
  const file = e.target.files[0]
  if (!file) return
  loading.value = true
  try {
    const fd = new FormData()
    fd.append('file', file)
    const { data } = await http.post('/plugins/install', fd, {
      headers: { 'Content-Type': 'multipart/form-data' },
      timeout: 120000
    })
    message.success(`插件 ${data.name} 安装成功`)
    load()
  } catch (err) {
    message.error(errMsg(err))
  } finally {
    loading.value = false
    e.target.value = ''
  }
}
async function act(p, op) {
  try {
    await http.post(`/plugins/${p.name}/${op}`)
    load()
  } catch (e) { message.error(errMsg(e)) }
}
const uninstallVisible = ref(false)
const uninstallTarget = ref(null)
const uninstalling = ref(false)
const purgeConfig = ref(false)
function askUninstall(p) {
  uninstallTarget.value = p
  purgeConfig.value = false
  uninstallVisible.value = true
}
async function confirmUninstall() {
  const p = uninstallTarget.value
  if (!p) return
  uninstalling.value = true
  try {
    const { data } = await http.delete(`/plugins/${p.name}`, { params: { purge_config: purgeConfig.value ? '1' : '0' } })
    message.success(`已卸载 ${p.name}（清理 ${data.removed_files} 个系统文件${data.config_kept ? '，配置已保留' : '，配置已删除'}）`)
    uninstallVisible.value = false
    load()
  } catch (e) {
    message.error(errMsg(e))
    uninstallVisible.value = false
  } finally { uninstalling.value = false }
}
async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/plugins')
    plugins.value = data.items
  } finally { loading.value = false }
}
onMounted(load)
</script>
