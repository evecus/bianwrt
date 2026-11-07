import { createRouter, createWebHistory } from 'vue-router'
import axios from 'axios'

const routes = [
  { path: '/login', component: () => import('../views/Login.vue') },
  {
    path: '/',
    component: () => import('../views/Layout.vue'),
    children: [
      { path: '', redirect: '/dashboard' },
      { path: 'dashboard', component: () => import('../views/Dashboard.vue') },
      { path: 'network', component: () => import('../views/Network.vue') },
      { path: 'firewall', component: () => import('../views/Firewall.vue') },
      { path: 'dhcp', component: () => import('../views/Dhcp.vue') },
      { path: 'packages', component: () => import('../views/Packages.vue') },
      { path: 'plugins', component: () => import('../views/Plugins.vue') },
      { path: 'plugin-page/:name', component: () => import('../views/PluginPage.vue') },
      { path: 'logs', component: () => import('../views/Logs.vue') },
      { path: 'processes', component: () => import('../views/Processes.vue') },
      { path: 'clients', component: () => import('../views/Clients.vue') },
      { path: 'services', component: () => import('../views/Services.vue') },
      { path: 'cron', component: () => import('../views/Cron.vue') },
      { path: 'backup', component: () => import('../views/Backup.vue') },
      { path: 'system', component: () => import('../views/System.vue') },
      { path: 'diagnostics', component: () => import('../views/Diagnostics.vue') },
      { path: 'routing', component: () => import('../views/Routing.vue') },
      { path: 'wireguard', component: () => import('../views/WireGuard.vue') },
      { path: 'dnsfilter', component: () => import('../views/DnsFilter.vue') }
    ]
  }
]

const router = createRouter({ history: createWebHistory(), routes })

router.beforeEach(async (to) => {
  // support ?token= for auto-login (e.g. after server restart)
  if (to.query.token) {
    localStorage.setItem('token', to.query.token)
    const { token, ...rest } = to.query
    return { path: to.path, query: rest }
  }
  const token = localStorage.getItem('token')
  if (to.path === '/login') return true
  if (!token) return '/login'
  try {
    await axios.get('/api/auth/status')
    return true
  } catch {
    return '/login'
  }
})

export default router
