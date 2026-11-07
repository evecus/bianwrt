<template>
  <div>
    <n-card title="菜单设置" style="margin-bottom: 16px">
      <template #header-extra>
        <n-space size="small">
          <n-button size="small" @click="reset">恢复默认</n-button>
          <n-button type="primary" size="small" :loading="saving" @click="save">保存配置</n-button>
        </n-space>
      </template>
      <n-alert type="info" :bordered="false" style="margin-bottom: 14px">
        可修改侧边栏品牌标题、分组标题、每个菜单项的标题与图标，以及显示 / 隐藏和顺序。
        已安装的插件页面会自动出现在「服务」分组，同样可以在这里改名、换图标或隐藏。
        配置保存在 settings 表中，会随「备份 / 恢复」一起保存。
      </n-alert>
      <n-spin :show="loading">
        <div class="grow">
          <span class="lbl">侧边栏标题</span>
          <n-input v-model:value="cfg.brand" size="small" style="width: 220px" placeholder="Bianwrt" />
          <span class="key">brand</span>
        </div>
        <n-card
          v-for="(g, gi) in cfg.groups"
          :key="g.key"
          size="small"
          style="margin-bottom: 14px"
        >
          <template #header>
            <span>分组 {{ gi + 1 }}</span>
            <n-tag size="small" style="margin-left: 8px">{{ g.items.length }} 项</n-tag>
          </template>
          <template #header-extra>
            <n-space size="small" align="center">
              <span class="muted">显示</span>
              <n-switch v-model:value="g.visible" size="small" />
              <n-button size="tiny" quaternary :disabled="gi === 0" @click="move(cfg.groups, gi, -1)">↑</n-button>
              <n-button
                size="tiny" quaternary
                :disabled="gi === cfg.groups.length - 1"
                @click="move(cfg.groups, gi, 1)"
              >↓</n-button>
            </n-space>
          </template>
          <div class="grow">
            <span class="lbl">分组标题</span>
            <n-input v-model:value="g.label" size="small" style="width: 200px" />
            <span class="key">{{ g.key }}</span>
          </div>
          <div class="mrow mrow-head">
            <span style="width: 18px"></span>
            <span class="col-label">菜单项标题</span>
            <span class="col-icon">图标</span>
            <span class="col-vis">显示</span>
            <span class="col-move">排序</span>
            <span class="key">路由 key</span>
          </div>
          <div class="mrow" v-for="(i, idx) in g.items" :key="i.key">
            <n-icon
              :component="iconComponents[i.icon] || iconComponents.ExtensionPuzzleOutline"
              size="18" style="flex: 0 0 18px"
            />
            <n-input v-model:value="i.label" size="small" class="col-label" placeholder="标题" />
            <n-select
              v-model:value="i.icon" :options="iconOptions" size="small"
              filterable class="col-icon"
            />
            <n-switch v-model:value="i.visible" size="small" class="col-vis" />
            <span class="col-move">
              <n-button size="tiny" quaternary :disabled="idx === 0" @click="move(g.items, idx, -1)">↑</n-button>
              <n-button
                size="tiny" quaternary
                :disabled="idx === g.items.length - 1"
                @click="move(g.items, idx, 1)"
              >↓</n-button>
            </span>
            <span class="key">{{ i.key }}</span>
          </div>
        </n-card>
      </n-spin>
    </n-card>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import {
  NCard, NButton, NInput, NSelect, NSwitch, NIcon, NTag, NSpace,
  NAlert, NSpin, useMessage, useDialog
} from 'naive-ui'
import http, { errMsg } from '../lib/http'
import { iconComponents, fallbackIcons, cloneMenu } from '../lib/menu'

const message = useMessage()
const dialog = useDialog()
const cfg = ref(cloneMenu(null))
const icons = ref(fallbackIcons)
const pluginKeys = ref([])
const saving = ref(false)
const loading = ref(false)

const iconOptions = computed(() => icons.value.map((i) => ({ label: i, value: i })))

function move(arr, i, d) {
  const j = i + d
  if (j < 0 || j >= arr.length) return
  const [x] = arr.splice(i, 1)
  arr.splice(j, 0, x)
}

async function load() {
  loading.value = true
  try {
    const { data } = await http.get('/menu/config')
    icons.value = data.icons?.length ? data.icons : fallbackIcons
    pluginKeys.value = data.plugin_keys || []
    const c = cloneMenu(data.config)
    // 允许存在的 key = 默认内置项 + 当前已安装的插件页面
    const allowed = new Set()
    ;(data.defaults?.groups || []).forEach((g) => g.items.forEach((i) => allowed.add(i.key)))
    pluginKeys.value.forEach((k) => allowed.add(k))
    c.groups.forEach((g) => { g.items = g.items.filter((i) => allowed.has(i.key)) })
    const declared = new Set()
    c.groups.forEach((g) => g.items.forEach((i) => declared.add(i.key)))
    const svc = c.groups.find((g) => g.key === 'grp-services')
    if (svc) {
      for (const k of pluginKeys.value) {
        if (!declared.has(k)) {
          svc.items.push({
            key: k,
            label: k.split('/').pop(),
            icon: 'ExtensionPuzzleOutline',
            visible: true
          })
        }
      }
    }
    cfg.value = c
  } catch (e) {
    message.error(String(errMsg(e)))
  } finally {
    loading.value = false
  }
}

async function save() {
  if (!String(cfg.value.brand || '').trim()) return message.error('侧边栏标题不能为空')
  for (const g of cfg.value.groups) {
    if (!String(g.label || '').trim()) return message.error('分组标题不能为空')
    for (const i of g.items) {
      if (!String(i.label || '').trim()) return message.error(`${i.key} 标题不能为空`)
    }
  }
  saving.value = true
  try {
    await http.put('/menu/config', cfg.value)
    message.success('菜单配置已保存，侧边栏已更新')
    window.dispatchEvent(new Event('menu-config-changed'))
  } catch (e) {
    message.error(String(errMsg(e)))
  } finally {
    saving.value = false
  }
}

function reset() {
  dialog.warning({
    title: '恢复默认菜单',
    content: '将清除自定义标题、图标、显隐与排序，恢复出厂菜单。确定继续？',
    positiveText: '恢复默认',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await http.post('/menu/reset')
        message.success('已恢复默认菜单')
        window.dispatchEvent(new Event('menu-config-changed'))
        await load()
      } catch (e) { message.error(String(errMsg(e))) }
    }
  })
}

onMounted(load)
</script>

<style scoped>
.grow { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; }
.lbl { color: #aaa; font-size: 12px; flex: 0 0 auto; }
.key { color: #888; font-size: 11px; font-family: monospace; margin-left: auto; white-space: nowrap; }
.muted { color: #888; font-size: 12px; }
.mrow {
  display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
  padding: 6px 0; border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}
.mrow-head { color: #888; font-size: 12px; border-bottom: 1px solid rgba(255, 255, 255, 0.12); }
.col-label { width: 180px; flex: 0 0 180px; }
.col-icon { width: 190px; flex: 0 0 190px; }
.col-vis { width: 48px; flex: 0 0 48px; }
.col-move { width: 72px; flex: 0 0 72px; }
</style>
