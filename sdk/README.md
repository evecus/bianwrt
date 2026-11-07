# bianwrt 官方插件 SDK

面向 `.rpkg` 插件页面开发。SDK 把面板 iframe 的 `postMessage` 桥封装成 Promise API，
让插件在**拿不到管理员 token** 的前提下安全地调用面板能力。

| 文件 | 用途 |
| --- | --- |
| `sdk/bianwrt-sdk.js` | 官方 SDK（零依赖，UMD，挂 `window.Bianwrt`） |
| `sdk/bianwrt-sdk.d.ts` | TypeScript 类型定义 |
| `examples/demo-hello/` | `static` 模式最小示例 |
| `examples/demo-rootfs/` | `rootfs/` + systemd 服务 + conffiles 示例 |
| `examples/demo-proxy/` | **`proxy` 反向代理模式 + SDK 完整示例** |
| `scripts/make-example-plugin.sh` | 一键打包全部示例为 `.rpkg` |

---

## 1. 快速开始

把 SDK 放进插件包的 `ui/` 目录，页面里引入后即可使用：

```html
<script src="./bianwrt-sdk.js"></script>
<script>
  Bianwrt.ready.then((info) => console.log('插件名', info.plugin, '主题', info.theme))

  // 调面板 API（父页面用已登录会话代发）
  Bianwrt.api.get('/api/system/stats').then((d) => console.log(d))

  // 持久化 KV 存储
  Bianwrt.storage.set('foo', { n: 1 }).then(() => Bianwrt.storage.get('foo'))
</script>
```

打包脚本会自动把 `sdk/bianwrt-sdk.js` 同步到示例插件的 `ui/`，**不要手改副本**：

```bash
./scripts/make-example-plugin.sh . demo-proxy
```

---

## 2. 运行环境与安全模型

插件页面由 `PluginPage.vue` 加载在 iframe 中：

```html
<iframe sandbox="allow-scripts allow-forms allow-popups allow-downloads"
        referrerpolicy="no-referrer">
```

**不含 `allow-same-origin`**，因此插件页是 opaque origin：

- 读不到主站的 `localStorage`（管理员 token 存在这里）→ 用 `Bianwrt.storage` 代替；
- `document.cookie` 不可用，面板下发的 `bwpt` 会话 cookie 是 `HttpOnly` 且 `Path=/plugins`；
- 直接 `fetch('/api/...')` 属于跨源请求 → 用 `Bianwrt.api.*` 代替；
- 插件进程**收不到** `token` 查询参数、`Authorization`、`Cookie`（代理层会剥离）。

父页面只代发以 `/api/` 开头的请求，且只允许 `GET/POST/PUT/DELETE/PATCH`。
静态资源请用**相对路径**（`./app.js`、`assets/x.css`），浏览器会自动带上
`/plugins/<name>/` 前缀；用 `/app.js` 会跳到面板根路径。

### 票据前缀（对插件透明）

opaque origin 的 iframe 不会随子资源请求发送 `SameSite=Lax` 会话 cookie，
所以面板在加载插件页时使用受限票据 URL：

```
/plugins/<name>/_bw/<ticket>/
```

`ticket = SHA256(name : session)`，是管理员会话 token 的**单向派生值**：
泄露票据既不能反推 token，也不能访问 `/api/` 管理接口，只能访问该插件自己的路径。
票据放在路径前缀里，因此 iframe 内所有相对路径资源（`<script>` / CSS / 图片）、
`Bianwrt.fetch()` 与代理重定向都会自动携带，无需插件代码感知；网关在转发给插件
后端之前会剥离这个前缀。**不要**把 `_bw` 用作插件自己的目录名。

---

## 3. API 参考

### `Bianwrt.ready` / `Bianwrt.info`

`ready: Promise<BianwrtInfo>`，桥就绪后 resolve；`info = { plugin, theme }`。
`Bianwrt.on('hello', fn)` 可监听父页面下发的初始化信息。

### `Bianwrt.api` — 面板 API

| 方法 | 说明 |
| --- | --- |
| `api.get(path, params?)` | GET，`path` 必须以 `/api/` 开头 |
| `api.post(path, data?)` | POST JSON |
| `api.put(path, data?)` / `api.patch(path, data?)` | 更新 |
| `api.delete(path, params?)` | DELETE |
| `api.request(method, path, { data, params })` | 通用 |

```js
const stats = await Bianwrt.api.get('/api/system/stats')
await Bianwrt.api.post('/api/diag/ping', { host: '1.1.1.1', count: 3 })
```

### `Bianwrt.storage` — 插件专属 KV 存储

数据由父页面存在 `localStorage` 的 `bw-plugin-kv:<plugin>` 命名空间，随备份一起保留。

| 方法 | 说明 |
| --- | --- |
| `storage.get(key)` | 取值，不存在返回 `null` |
| `storage.set(key, value)` | 写入（JSON 可序列化，≤64KB） |
| `storage.remove(key)` / `storage.clear()` | 删除 / 清空 |
| `storage.keys()` | 列出所有键 |

键名仅允许 `A-Z a-z 0-9 _ . : -`，长度 ≤64。

### `Bianwrt.notify` — 面板全局提示

```js
Bianwrt.notify.success('保存成功')
Bianwrt.notify.error('操作失败')
Bianwrt.notify('warning', '注意')
```

### `Bianwrt.pluginUrl()` / `Bianwrt.fetch()` — 插件自身后端

仅 `proxy` 模式有意义。面板给代理响应加了 `Access-Control-Allow-Origin: *`，
所以 opaque origin 的页面也能 fetch 自己的后端。`pluginUrl()` 以当前文档目录为基准，
自动保留票据前缀（见上节），因此传相对路径即可：

```js
const r = await Bianwrt.fetch('api/state')       // → /plugins/<name>/api/state
const d = await r.json()
```

---

## 4. 两种页面模式

### `static`（静态托管）

包内提供 `ui/index.html`，面板直接读文件返回，无需后台进程：

```toml
[package]
name = "my-plugin"
version = "1.0.0"
web_mode = "static"
```

### `proxy`（反向代理，推荐给有后端的插件）

插件自己起一个 HTTP 服务，只监听 `127.0.0.1:<port>`；面板把
`/plugins/<name>/...` 转发到 `http://127.0.0.1:<port>/...`：

```toml
[package]
name = "my-proxy"
version = "1.0.0"
web_mode = "proxy"

[web]
port = 19090        # 必须 ≥1024
timeout = 15        # 单次请求超时，1~120 秒

[service]
exec = "bin/server.py"       # 相对包根，注册为 bianwrt-plugin-my-proxy.service
args = ["--port", "19090"]
```

代理层的行为：

- 剥离 `/plugins/<name>` 前缀，剔除 `token` 参数，转发其余 query；
- 转发请求方法、请求体（≤32MB）与非敏感头，附加 `X-Bianwrt-Plugin` / `X-Forwarded-Proto`；
- 丢弃 `Set-Cookie`（避免覆盖面板会话 cookie）、重写相对 `Location`；
- 解码 chunked 响应、限制响应 ≤32MB、整体超时；
- 为响应补 CORS 头，OPTIONS 预检直接返回 204；
- 只连 `127.0.0.1`，不跟随重定向，插件进程拿不到管理员凭据。

> 插件后端返回的重定向要用**相对路径**（如 `./login`），或根路径 `/foo`
> （面板会自动改写为 `/plugins/<name>/foo`）。

---

## 5. manifest.toml 完整规范

```toml
[package]
name = "my-plugin"          # 小写字母 / 数字 / -，≤64
version = "1.0.0"           # 不允许静默降级，需先卸载再装旧版
description = "一句话描述"
author = "you"
homepage = "https://..."
web_mode = "static"         # static | proxy | none（none=纯系统插件，无页面）
conffiles = ["etc/my-plugin/conf.json"]  # 升级 / 卸载时保留用户配置

[service]                   # 可选：注册 systemd 服务
exec = "bin/my-plugin"
args = ["--port", "9090"]
user = "root"
workdir = ""                # 默认为插件版本目录

[web]                       # web_mode=proxy 时必填
port = 19090
timeout = 15

[deps]                      # 可选：依赖自动安装
apt = ["curl", "python3"]   # Debian/Ubuntu（apk 系统复用此列表）
opkg = ["curl"]             # OpenWrt
commands = ["curl"]         # 安装后必须在 PATH 中存在的命令
```

包内可选文件：

| 文件 | 作用 |
| --- | --- |
| `SHA256SUMS` | 逐文件校验和，写了就必须全部匹配 |
| `manifest.toml.asc` | GPG detached 签名；`BIANWRT_REQUIRE_SIGNED=1` 时强制 |
| `rootfs/` | 完整根目录结构，安装时铺到 `/` |
| `ui/` | 静态页面（`static`） |
| `bin/` 等 | 插件自带程序（`proxy` / `service`） |

### 依赖自动安装

安装前先检查 `dpkg-query` / `opkg status` / `apk info`，缺失的包由面板调用
`apt-get install -y --no-install-recommends`（或 opkg/apk）自动安装，超时 300s；
随后校验 `deps.commands` 是否可用。依赖检查在任何文件写入之前完成，失败即中止安装。

- `BIANWRT_AUTO_DEPS=0`：关闭自动安装，退化为「只校验并报错」。
- 自动安装的**系统包不会随插件卸载而移除**，也不会因后续安装步骤失败而回滚
  （自动卸载系统包可能破坏依赖链）；错误信息会列出本次新装的包，便于手动清理。
- 只接受白名单字符的包名，命令用 argv 数组执行，不经 shell。

### 其他安全约束

所有 manifest 字段禁止控制字符、换行、引号（防 systemd unit 注入）；
禁止绝对路径与 `..` 路径穿越；解压限制 64MB 上传 / 256MB 展开 / 4096 条目 / 单文件 64MB；
安装事务化，任一步失败回滚 `rootfs`、插件目录、`current`、unit 与数据库记录。

---

## 6. 环境变量

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `BIANWRT_WORK_DIR` | `/etc/bianwrt` | `data/`/`plugins/`/`conf/` 根目录 |
| `BIANWRT_PORT` | `8088` | 面板监听端口 |
| `BIANWRT_AUTO_DEPS` | `1` | `0` 关闭依赖自动安装 |
| `BIANWRT_REQUIRE_SIGNED` | `0` | `1` 强制只装带 GPG 签名的包 |

---

## 7. 打包与安装

```bash
tar czf my-plugin.rpkg -C my-plugin-dir manifest.toml ui/ bin/
# 或直接用示例脚本
./scripts/make-example-plugin.sh . demo-proxy
```

登录面板 → **插件中心** → 安装 `.rpkg` 包。安装成功后：

- 页面挂载于 `/plugins/<name>/`（需登录）；
- 带页面的插件自动出现在左侧「服务」分组，可在**系统 → 菜单设置**里改名 / 换图标 / 隐藏；
- systemd 服务名 `bianwrt-plugin-<name>.service`；
- 备份 / 恢复会一并打包插件目录、文件清单、systemd unit 与运行期元数据。
