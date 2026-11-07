import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { create } from 'naive-ui'
import App from './App.vue'
import router from './router'
import './style.css'

const app = createApp(App)
app.use(createPinia())
app.use(create())
app.use(router)
app.mount('#app')
