<template>
  <div style="height: 100vh; display: flex; align-items: center; justify-content: center">
    <n-card style="width: 380px" :title="needSetup ? '初始化管理员' : 'Bianwrt 登录'">
      <n-form>
        <n-form-item label="用户名">
          <n-input v-model:value="username" placeholder="admin" />
        </n-form-item>
        <n-form-item label="密码">
          <n-input v-model:value="password" type="password" show-password-on="click" @keydown.enter="submit" />
        </n-form-item>
        <n-button type="primary" block :loading="busy" @click="submit">
          {{ needSetup ? '创建管理员并登录' : '登录' }}
        </n-button>
      </n-form>
    </n-card>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { NCard, NForm, NFormItem, NInput, NButton, useMessage } from 'naive-ui'
import http, { errMsg } from '../lib/http'

const router = useRouter()
const message = useMessage()
const needSetup = ref(false)
const username = ref('admin')
const password = ref('')
const busy = ref(false)

onMounted(async () => {
  try {
    const { data } = await http.get('/auth/status')
    needSetup.value = data.need_setup
  } catch { /* ignore */ }
})

async function submit() {
  if (!username.value || !password.value) return message.error('请输入用户名和密码')
  busy.value = true
  try {
    if (needSetup.value) {
      await http.post('/auth/setup', { username: username.value, password: password.value })
    }
    const { data } = await http.post('/auth/login', { username: username.value, password: password.value })
    localStorage.setItem('token', data.token)
    router.push('/dashboard')
  } catch (e) {
    message.error(errMsg(e))
  } finally {
    busy.value = false
  }
}
</script>
