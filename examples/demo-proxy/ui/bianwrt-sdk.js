/*!
 * bianwrt 官方插件 SDK v1.0.0
 * 适用：插件页面运行在面板的 sandbox iframe 中（opaque origin）。
 * 用法：<script src="./bianwrt-sdk.js"></script> 后使用全局 Bianwrt。
 *
 * 能力：调用面板 API / 持久化 KV 存储 / 全局通知 / 插件自身后端请求。
 * 依赖父页面 PluginPage.vue 的 postMessage 桥（父页面持有已登录会话）。
 */
(function (global) {
  'use strict'

  var seq = 0
  var pending = Object.create(null)
  var listeners = Object.create(null)
  var TIMEOUT = 30000

  function inferPlugin() {
    var m = String(global.location ? global.location.pathname : '').match(/^\/plugins\/([^/]+)/)
    if (!m) return ''
    try { return decodeURIComponent(m[1]) } catch (e) { return m[1] }
  }

  var info = { plugin: inferPlugin(), theme: 'dark' }
  var readyResolve
  var ready = new Promise(function (res) { readyResolve = res })

  function emit(evt, data) {
    var fns = listeners[evt] || []
    for (var i = 0; i < fns.length; i++) {
      try { fns[i](data) } catch (e) { /* 插件回调异常不影响桥 */ }
    }
  }

  function call(method, extra) {
    return new Promise(function (resolve, reject) {
      if (!global.parent || global.parent === global) {
        return reject(new Error('Bianwrt SDK 必须在面板插件 iframe 内运行'))
      }
      var id = ++seq
      var timer = global.setTimeout(function () {
        delete pending[id]
        reject(new Error('Bianwrt SDK 请求超时（30s）'))
      }, TIMEOUT)
      pending[id] = { resolve: resolve, reject: reject, timer: timer }
      var msg = { __bianwrt: 1, id: id, method: method }
      if (extra) {
        for (var k in extra) {
          if (Object.prototype.hasOwnProperty.call(extra, k)) msg[k] = extra[k]
        }
      }
      try {
        global.parent.postMessage(msg, '*')
      } catch (e) {
        delete pending[id]
        global.clearTimeout(timer)
        reject(e)
      }
    })
  }

  global.addEventListener('message', function (ev) {
    var m = ev.data
    if (!m || m.__bianwrt !== 1) return
    if (m.type === 'hello') {
      info = {
        plugin: m.plugin || info.plugin,
        theme: m.theme || info.theme
      }
      emit('hello', info)
      readyResolve(info)
      return
    }
    if (typeof m.id === 'undefined') return
    var p = pending[m.id]
    if (!p) return
    delete pending[m.id]
    global.clearTimeout(p.timer)
    if (m.ok) p.resolve(m.data)
    else p.reject(new Error(m.error || String(m.data || '请求失败')))
  })

  // 主动探测：兼容 hello 广播早于 SDK 监听器注册的情况
  call('PING').then(function (d) {
    if (d && typeof d === 'object') info = Object.assign(info, d)
    emit('hello', info)
    readyResolve(info)
  }).catch(function () {
    // 不在面板 iframe 内时仍 resolve，页面可自行降级展示
    readyResolve(info)
  })

  function request(method, path, opts) {
    opts = opts || {}
    if (typeof path !== 'string' || path.indexOf('/api/') !== 0) {
      return Promise.reject(new Error('path 必须以 /api/ 开头，例如 /api/system/stats'))
    }
    return call(String(method).toUpperCase(), {
      path: path,
      data: opts.data,
      params: opts.params
    })
  }

  var api = {
    request: request,
    get: function (path, params) { return request('GET', path, { params: params }) },
    post: function (path, data) { return request('POST', path, { data: data }) },
    put: function (path, data) { return request('PUT', path, { data: data }) },
    patch: function (path, data) { return request('PATCH', path, { data: data }) },
    delete: function (path, params) { return request('DELETE', path, { params: params }) }
  }

  var storage = {
    get: function (key) { return call('STORAGE_GET', { key: key }) },
    set: function (key, value) { return call('STORAGE_SET', { key: key, value: value }) },
    remove: function (key) { return call('STORAGE_DEL', { key: key }) },
    keys: function () { return call('STORAGE_KEYS') },
    clear: function () { return call('STORAGE_CLEAR') }
  }

  function notify(type, text) {
    return call('NOTIFY', { type: type, text: text })
  }
  notify.success = function (t) { return notify('success', t) }
  notify.info = function (t) { return notify('info', t) }
  notify.warning = function (t) { return notify('warning', t) }
  notify.error = function (t) { return notify('error', t) }

  function pluginUrl(path) {
    // 以当前文档所在目录为基准解析：自动保留 /_bw/<ticket>/ 票据前缀，
    // 否则插件自身后端的 fetch 会因缺少票据被面板 gate 拒绝。
    var href = String(global.location ? global.location.href : '')
    var q = href.indexOf('?')
    if (q >= 0) href = href.slice(0, q)
    var h = href.indexOf('#')
    if (h >= 0) href = href.slice(0, h)
    if (href.slice(-1) !== '/') href = href.replace(/[^/]*$/, '')
    if (!href) href = '/plugins/' + encodeURIComponent(info.plugin || '') + '/'
    if (!path) return href
    try {
      return new URL(String(path).replace(/^\/+/, ''), href).href
    } catch (e) {
      return href + String(path).replace(/^\/+/, '')
    }
  }

  function backendFetch(path, opts) {
    return global.fetch(pluginUrl(path), opts)
  }

  function on(evt, fn) {
    if (typeof fn !== 'function') return function () {}
    (listeners[evt] || (listeners[evt] = [])).push(fn)
    return function () { off(evt, fn) }
  }
  function off(evt, fn) {
    var a = listeners[evt]
    if (!a) return
    var i = a.indexOf(fn)
    if (i >= 0) a.splice(i, 1)
  }

  var Bianwrt = {
    version: '1.0.0',
    ready: ready,
    get info() { return info },
    api: api,
    storage: storage,
    notify: notify,
    pluginUrl: pluginUrl,
    fetch: backendFetch,
    on: on,
    off: off
  }

  global.Bianwrt = Bianwrt
  if (typeof module !== 'undefined' && module.exports) module.exports = Bianwrt
})(typeof window !== 'undefined' ? window : this)
