import { createApp } from 'vue'
import { createPinia } from 'pinia'
import naive from 'naive-ui'
import App from './App.vue'
import router from './router'
import './style.css'

const app = createApp(App)
app.use(createPinia())
// 全局注册所有 naive-ui 组件：页面模板里直接写 <n-card> / <n-data-table> 等，
// 无需在每个 SFC 中逐个 import（create() 默认不注册任何组件，切勿用它做全局注册）。
app.use(naive)
app.use(router)
app.mount('#app')
