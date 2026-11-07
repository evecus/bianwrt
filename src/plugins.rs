//! 插件系统：.rpkg 包安装 / 卸载 / 服务托管 / 静态页面托管。
//!
//! 安全约束：
//! - 上传流式限流，解压限条目数 / 总字节 / 单文件大小；
//! - 包内可选 SHA256SUMS 自校验、可选 GPG detached 签名；
//! - manifest 全字段禁止控制字符与换行，exec 相对路径不得逃逸插件目录；
//! - 安装事务化：失败回滚 rootfs / 插件目录 / current / unit；
//! - 插件静态页需要登录（gate 中间件），页面运行在无同源凭据的 sandbox iframe 中。

use axum::body::Body;
use axum::extract::multipart::Multipart;
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tar::Archive;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::state::{get_setting, set_setting, AppState};
use crate::cmd;

const MAX_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
const MAX_SINGLE: u64 = 64 * 1024 * 1024;
const MAX_PROXY_BODY: usize = 32 * 1024 * 1024;
const WEB_MODES: [&str; 3] = ["static", "none", "proxy"];

#[derive(Deserialize, Clone)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default = "default_web_mode")]
    pub web_mode: String,
    /// rootfs 内的用户配置文件路径（相对 rootfs/），升级与卸载时保留不覆盖
    #[serde(default)]
    pub conffiles: Vec<String>,
}
fn default_web_mode() -> String {
    "static".into()
}

#[derive(Deserialize, Clone)]
pub struct ServiceMeta {
    pub exec: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub workdir: String,
}

#[derive(Deserialize, Clone)]
pub struct DepsMeta {
    #[serde(default)]
    pub apt: Vec<String>,
    /// OpenWrt / opkg 包名（可选）
    #[serde(default)]
    pub opkg: Vec<String>,
    /// 安装后必须存在的可执行命令（纯校验，不安装）
    #[serde(default)]
    pub commands: Vec<String>,
}

/// web_mode=proxy 时的反向代理配置：插件自己监听 127.0.0.1:<port>。
#[derive(Deserialize, Clone)]
pub struct WebMeta {
    pub port: u16,
    /// 单次请求超时（秒），默认 15，范围 1~120
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}
fn default_timeout() -> u64 {
    15
}

#[derive(Deserialize, Clone)]
pub struct Manifest {
    pub package: PackageMeta,
    pub service: Option<ServiceMeta>,
    pub deps: Option<DepsMeta>,
    pub web: Option<WebMeta>,
}

/// 插件运行期元数据（代理端口 / 依赖安装记录），存 settings.plugin_meta.<name>。
fn meta_key(name: &str) -> String {
    format!("plugin_meta.{}", name)
}
fn load_meta(state: &AppState, name: &str) -> serde_json::Value {
    let db = state.db.lock().unwrap();
    get_setting(&db, &meta_key(name))
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}
fn save_meta(state: &AppState, name: &str, v: &serde_json::Value) {
    let db = state.db.lock().unwrap();
    set_setting(&db, &meta_key(name), &v.to_string());
}

fn unit_name(name: &str) -> String {
    format!("bianwrt-plugin-{}.service", name)
}

async fn systemctl(args: &[&str]) -> bool {
    tokio::process::Command::new("systemctl")
        .args(args)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn sysctl_active(unit: &str) -> (bool, bool) {
    let out = tokio::process::Command::new("systemctl")
        .args(["is-active", unit])
        .output()
        .await;
    let active = out.map(|o| o.status.success()).unwrap_or(false);
    let exists = tokio::process::Command::new("systemctl")
        .args(["cat", unit])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    (exists, active)
}

// ── P0-1：插件静态页鉴权 ────────────────────────────────────────────

/// 从 Authorization / ?token= / scoped cookie 中取会话 token。
fn plugin_token(req: &Request) -> Option<String> {
    if let Some(t) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(t.to_string());
    }
    if let Some(t) = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
    {
        return Some(t.to_string());
    }
    let cookies = req.headers().get(header::COOKIE)?.to_str().ok()?;
    cookies
        .split(';')
        .find_map(|c| c.trim().strip_prefix("bwpt="))
        .map(|s| s.to_string())
}

// ── 插件页面票据（opaque origin iframe 无 cookie 问题的解法） ──────────
//
// sandbox iframe 不带 allow-same-origin → opaque origin，SameSite=Lax 的
// HttpOnly 会话 cookie 不会随插件子资源请求发送（真实 HTTP 部署也无法靠
// SameSite=None;Secure 解决，因为 Secure 需要 HTTPS）。因此改为在 URL 路径中
// 携带「受限票据」：ticket = SHA256(name : session)，是会话 token 的单向派生，
// 泄露也不能反推 token；gate 只认它访问 /plugins/<name>/，不能用于 /api/。
// 票据放在路径前缀 /_bw/<ticket>/ 中，iframe 内所有相对路径资源、fetch
// 与重定向都会自动带上，无需插件代码感知。

const TICKET_SEG: &str = "_bw";

fn plugin_ticket(name: &str, session: &str) -> String {
    hex::encode(Sha256::digest(
        format!("bwpt:{}:{}", name, session).as_bytes(),
    ))
}

/// 从 /plugins/<name>/_bw/<ticket>/... 中取出 (name, ticket)。
fn ticket_of_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/plugins/")?;
    let mut it = rest.split('/');
    let name = it.next()?.to_string();
    if it.next() != Some(TICKET_SEG) {
        return None;
    }
    let t = it.next()?.to_string();
    if t.is_empty() {
        return None;
    }
    Some((name, t))
}

/// 插件内部路径去掉票据前缀（_bw/<ticket>/ 之后才是真正的插件路径）。
fn strip_ticket_prefix(rest: &str) -> &str {
    let mut it = rest.splitn(3, '/');
    if it.next() == Some(TICKET_SEG) {
        let _ = it.next();
        return it.next().unwrap_or("");
    }
    rest
}

/// 票据是否有效：存在某个有效会话，其派生值与该插件票据匹配。
fn ticket_valid(state: &AppState, name: &str, ticket: &str) -> bool {
    let Ok(sessions) = state.sessions.try_lock() else {
        return false;
    };
    sessions
        .keys()
        .any(|s| plugin_ticket(name, s) == ticket)
}

/// 为当前登录会话签发指定插件的页面票据（供 PluginPage.vue 构造 iframe URL）。
pub async fn ticket(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    req: Request,
) -> Response {
    if !valid_plugin_name(&name) {
        return cmd::err("非法插件名");
    }
    let mode: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT web_mode FROM plugins WHERE name=?1",
            [&name],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    if mode.is_none() || mode.as_deref() == Some("none") {
        return cmd::err("插件不存在或为无页面模式");
    }
    let Some(session) = plugin_token(&req) else {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    };
    let valid = state
        .sessions
        .try_lock()
        .ok()
        .map(|m| m.contains_key(session.as_str()))
        .unwrap_or(false);
    if !valid {
        return (StatusCode::UNAUTHORIZED, "会话已过期").into_response();
    }
    let t = plugin_ticket(&name, &session);
    Json(json!({
        "ticket": t,
        "url": format!("/plugins/{}/_bw/{}/", name, t),
    }))
    .into_response()
}

/// plugin_pages 路由专用鉴权中间件。首次通过 ?token= 访问时下发
/// HttpOnly + Path=/plugins 的 scoped cookie，插件子资源请求自动携带，
/// 页面 JS 读不到（且 iframe 为 opaque origin，见 PluginPage.vue）。
pub async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // CORS 预检（sandbox iframe 的 opaque origin 发起）不携带 cookie，
    // 放行给插件处理器返回 204；真实请求仍必须携带有效会话。
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let token = plugin_token(&req);
    let from_query = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .map(|s| s.to_string());
    let by_token = token
        .as_ref()
        .and_then(|t| {
            state
                .sessions
                .try_lock()
                .ok()
                .map(|m| m.contains_key(t.as_str()))
        })
        .unwrap_or(false);
    // 票据路径：opaque origin iframe 内的子资源 / fetch 不带 cookie，靠 URL 前缀鉴权
    let by_ticket = ticket_of_path(req.uri().path())
        .map(|(n, t)| ticket_valid(&state, &n, &t))
        .unwrap_or(false);
    let ok = by_token || by_ticket;
    if !ok {
        return (StatusCode::UNAUTHORIZED, "未登录或会话已过期：插件页面需要登录").into_response();
    }
    let mut resp = next.run(req).await;
    // 仅当会话 token 本身就来自 ?token=（首次导航）且有效时才下发 scoped cookie，
    // 避免「已带 Authorization 又附任意 ?token=」的请求用无效值覆盖已有 bwpt cookie。
    if by_token {
        if let Some(t) = from_query {
            if token.as_deref() == Some(t.as_str()) {
                if let Ok(v) = HeaderValue::from_str(&format!(
                    "bwpt={}; Path=/plugins; HttpOnly; SameSite=Lax; Max-Age=604800",
                    t
                )) {
                    resp.headers_mut().append(header::SET_COOKIE, v);
                }
            }
        }
    }
    resp
}

// ── 列表 ────────────────────────────────────────────────────────────

pub async fn list(State(state): State<AppState>) -> Response {
    let rows: Vec<(String, String, String, i64, String, String, String, String)> = {
        let db = state.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT name,version,web_mode,has_service,description,author,homepage,installed_at FROM plugins ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
            ))
        })
        .unwrap()
        .filter_map(|x| x.ok())
        .collect()
    };
    let mut items = Vec::new();
    for (name, version, web_mode, has_service, description, author, homepage, installed_at) in rows {
        let unit = unit_name(&name);
        let (exists, active) = if has_service == 1 {
            sysctl_active(&unit).await
        } else {
            (false, false)
        };
        let (checksum, signed) = {
            let db = state.db.lock().unwrap();
            (
                get_setting(&db, &format!("plugin_sha.{}", name)).unwrap_or_default(),
                get_setting(&db, &format!("plugin_signed.{}", name))
                    .map(|v| v == "1")
                    .unwrap_or(false),
            )
        };
        let meta = load_meta(&state, &name);
        let proxy_port = meta.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
        let deps = meta.get("deps").cloned().unwrap_or_else(|| json!({}));
        let has_ui = (web_mode == "proxy" && proxy_port > 0)
            || state
                .plugins_dir
                .join(&name)
                .join("current")
                .join("ui")
                .join("index.html")
                .is_file();
        items.push(json!({
            "name": name, "version": version, "web_mode": web_mode,
            "has_service": has_service == 1, "service_exists": exists, "active": active,
            "description": description, "author": author, "homepage": homepage,
            "installed_at": installed_at, "has_ui": has_ui,
            "checksum": checksum, "signed": signed,
            "proxy_port": proxy_port, "deps": deps,
            "url": format!("/plugins/{}/", name),
        }));
    }
    Json(json!({"items": items})).into_response()
}

// ── 包解析与校验 ────────────────────────────────────────────────────

struct PkgEntry {
    rel: PathBuf,
    content: Vec<u8>,
    is_dir: bool,
    mode: u32,
}

/// 解析 tar.gz 到内存：路径穿越防护 + 解压炸弹限制 + 兼容单一顶层目录。
fn parse_archive(bytes: &[u8]) -> Result<Vec<PkgEntry>, String> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
    let entries = archive.entries().map_err(|e| format!("包读取失败: {}", e))?;
    let mut all: Vec<PkgEntry> = Vec::new();
    let mut roots: Vec<Option<String>> = Vec::new();
    let mut total: u64 = 0;
    for e in entries {
        let mut e = e.map_err(|e| format!("包损坏: {}", e))?;
        if all.len() >= MAX_ENTRIES {
            return Err(format!("包内条目超过 {} 个上限", MAX_ENTRIES));
        }
        let path = e.path().map_err(|e| e.to_string())?.to_path_buf();
        let p = path.to_string_lossy();
        if path.is_absolute() || p.contains("..") {
            return Err(format!("包内存在非法路径: {}", p));
        }
        let size = e.header().size().unwrap_or(0);
        if size > MAX_SINGLE {
            return Err(format!("包内单个文件超过 {}MB: {}", MAX_SINGLE / 1024 / 1024, p));
        }
        total = total.saturating_add(size);
        if total > MAX_UNPACKED {
            return Err(format!("解压后总大小超过 {}MB 上限", MAX_UNPACKED / 1024 / 1024));
        }
        // 规范化掉开头的 `./`（`tar -C dir .` 常见产物），否则会干扰单一顶层目录判定
        let comps: Vec<_> = path
            .components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect();
        let root: Option<String> = comps
            .first()
            .filter(|_| comps.len() > 1)
            .map(|c| c.as_os_str().to_string_lossy().to_string());
        roots.push(root);
        let is_dir = e.header().entry_type().is_dir();
        let mode = e.header().mode().unwrap_or(0o644);
        let mut content = Vec::new();
        if !is_dir {
            use std::io::Read;
            e.read_to_end(&mut content).map_err(|e| e.to_string())?;
        }
        all.push(PkgEntry {
            rel: comps.iter().collect(),
            content,
            is_dir,
            mode,
        });
    }
    let manifest_at_root = all.iter().any(|x| x.rel == Path::new("manifest.toml"));
    let first_root: Option<String> = roots.first().cloned().flatten();
    let has_root = !manifest_at_root
        && !roots.is_empty()
        && roots.iter().all(|r| r.is_some())
        && first_root
            .as_ref()
            .map(|r0| roots.iter().all(|r| r.as_deref() == Some(r0.as_str())))
            .unwrap_or(false);
    if has_root {
        for x in all.iter_mut() {
            x.rel = x.rel.components().skip(1).collect();
        }
    }
    Ok(all)
}

/// 包内可选 SHA256SUMS：逐项校验，列了就必须全部匹配。
fn verify_checksums(entries: &[PkgEntry]) -> Result<bool, String> {
    let Some(sums) = entries.iter().find(|x| x.rel == Path::new("SHA256SUMS")) else {
        return Ok(false);
    };
    let text = String::from_utf8_lossy(&sums.content);
    let mut checked = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next().unwrap_or("").to_ascii_lowercase();
        let rel = parts.next().unwrap_or("");
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("SHA256SUMS 格式错误（应为 <64位hex>  <相对路径>）".into());
        }
        if !cmd::safe_rel(rel, 512) {
            return Err(format!("SHA256SUMS 含非法路径: {}", rel));
        }
        let Some(e) = entries
            .iter()
            .find(|x| !x.is_dir && x.rel == Path::new(rel))
        else {
            return Err(format!("SHA256SUMS 中的文件不存在: {}", rel));
        };
        if hex::encode(Sha256::digest(&e.content)) != hash {
            return Err(format!("校验和不匹配（包可能损坏或被篡改）: {}", rel));
        }
        checked += 1;
    }
    if checked == 0 {
        return Err("SHA256SUMS 为空".into());
    }
    Ok(true)
}

/// 可选 GPG detached 签名（manifest.toml.asc）。
/// BIANWRT_REQUIRE_SIGNED=1 时强制要求签名包。
async fn verify_signature(
    state: &AppState,
    entries: &[PkgEntry],
    manifest_raw: &[u8],
) -> Result<bool, String> {
    let require = std::env::var("BIANWRT_REQUIRE_SIGNED")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    let sig = entries.iter().find(|x| {
        x.rel == Path::new("manifest.toml.asc") || x.rel == Path::new("manifest.toml.sig")
    });
    let Some(s) = sig else {
        if require {
            return Err("本机要求签名插件包，但包内缺少 manifest.toml.asc".into());
        }
        return Ok(false);
    };
    if !cmd::which("gpg") {
        return Err("包带 GPG 签名，但系统未安装 gpg，无法校验".into());
    }
    let dir = state.data_dir.join(format!("sigcheck-{}", cmd::stamp()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mf = dir.join("manifest.toml");
    let sf = dir.join("manifest.toml.asc");
    let mfs = mf.to_string_lossy().to_string();
    let sfs = sf.to_string_lossy().to_string();
    let write = std::fs::write(&mf, manifest_raw).and_then(|_| std::fs::write(&sf, &s.content));
    if let Err(e) = write {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("签名校验准备失败: {}", e));
    }
    let out = cmd::run("gpg", &["--verify", sfs.as_str(), mfs.as_str()], 15).await;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.ok {
        return Err(format!("GPG 签名校验失败: {}", out.stderr.trim()));
    }
    Ok(true)
}

/// manifest 全字段校验：注入防护 + 枚举 + 路径逃逸防护。
fn validate_manifest(m: &Manifest) -> Result<(), String> {
    let p = &m.package;
    if p.name.is_empty()
        || p.name.len() > 64
        || !p
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("插件名仅允许小写字母/数字/连字符，且不超过 64 字符".into());
    }
    if p.version.is_empty()
        || p.version.len() > 32
        || !p
            .version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
    {
        return Err("版本号格式非法（仅允许字母数字与 . - +）".into());
    }
    if !WEB_MODES.contains(&p.web_mode.as_str()) {
        return Err("web_mode 仅支持 static / none / proxy".into());
    }
    if p.web_mode == "proxy" {
        let Some(w) = &m.web else {
            return Err("web_mode=proxy 必须提供 [web] 段与 port（插件本机监听端口）".into());
        };
        if w.port < 1024 {
            return Err("proxy 端口不得使用 1024 以下的特权端口".into());
        }
        if !(1..=120).contains(&w.timeout) {
            return Err("web.timeout 须在 1~120 秒之间".into());
        }
    }
    for (label, s) in [
        ("description", &p.description),
        ("author", &p.author),
        ("homepage", &p.homepage),
    ] {
        if !cmd::safe_manifest_text(s, 512) {
            return Err(format!("{} 含控制字符/引号或过长（>512）", label));
        }
    }
    for c in &p.conffiles {
        if !cmd::safe_rel(c, 512) {
            return Err(format!("conffiles 路径非法: {}", c));
        }
    }
    if let Some(d) = &m.deps {
        for (kind, list) in [("apt", &d.apt), ("opkg", &d.opkg)] {
            for a in list {
                if a.is_empty()
                    || a.len() > 128
                    || !a
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.:".contains(c))
                {
                    return Err(format!("{} 依赖名非法: {}", kind, a));
                }
            }
        }
        for c in &d.commands {
            if c.is_empty()
                || c.len() > 64
                || c.contains('/')
                || !c
                    .chars()
                    .all(|x| x.is_ascii_alphanumeric() || "_-.".contains(x))
            {
                return Err(format!("deps.commands 命令名非法: {}", c));
            }
        }
    }
    if let Some(s) = &m.service {
        if s.exec.is_empty() || !cmd::safe_manifest_text(&s.exec, 512) || s.exec.contains("..") {
            return Err("service.exec 含非法字符或路径穿越".into());
        }
        if !Path::new(&s.exec).is_absolute() && !cmd::safe_rel(&s.exec, 512) {
            return Err("service.exec 相对路径必须落在插件目录内".into());
        }
        for a in &s.args {
            if !cmd::safe_manifest_text(a, 1024) {
                return Err("service.args 含控制字符/换行/引号（禁止注入 systemd unit）".into());
            }
        }
        if !s.user.is_empty()
            && (s.user.len() > 64
                || !s
                    .user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)))
        {
            return Err("service.user 仅允许字母数字/下划线/连字符".into());
        }
        if !s.workdir.is_empty()
            && (!cmd::safe_manifest_text(&s.workdir, 512)
                || s.workdir.chars().any(|c| c.is_whitespace()))
        {
            return Err("service.workdir 含非法字符或空白".into());
        }
    }
    Ok(())
}

/// 语义化版本比较（数字段逐段比较），用于禁止静默降级。
fn cmp_version(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (va, vb) = (parse(a), parse(b));
    for i in 0..va.len().max(vb.len()) {
        let o = va.get(i).copied().unwrap_or(0).cmp(&vb.get(i).copied().unwrap_or(0));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// systemd ExecStart 参数转义（双引号内反斜杠与引号转义）。
fn sd_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}

// ── 依赖自动安装 ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum DepManager {
    Apt,
    Opkg,
    Apk,
    None,
}

fn detect_manager() -> DepManager {
    if cmd::which("apt-get") {
        DepManager::Apt
    } else if cmd::which("opkg") {
        DepManager::Opkg
    } else if cmd::which("apk") {
        DepManager::Apk
    } else {
        DepManager::None
    }
}

async fn pkg_installed(mgr: DepManager, pkg: &str) -> bool {
    match mgr {
        DepManager::Apt => {
            let out = cmd::run("dpkg-query", &["-W", "-f=${db:Status-Abbrev}", pkg], 10).await;
            out.ok && out.stdout.trim().starts_with("ii")
        }
        DepManager::Opkg => {
            let out = cmd::run("opkg", &["status", pkg], 10).await;
            out.ok && out.stdout.contains("Status: install ok installed")
        }
        DepManager::Apk => cmd::run("apk", &["info", "-e", pkg], 10).await.ok,
        DepManager::None => false,
    }
}

/// 带 DEBIAN_FRONTEND=noninteractive 的包安装（不经 shell、带超时与 kill_on_drop）。
async fn pkg_install(program: &str, args: &[&str], secs: u64) -> cmd::Out {
    let mut c = tokio::process::Command::new(program);
    c.args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(secs), c.output()).await {
        Ok(Ok(o)) => cmd::Out {
            ok: o.status.success(),
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Ok(Err(e)) => cmd::Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("无法执行 {}: {}", program, e),
        },
        Err(_) => cmd::Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("包安装超时（{}s）", secs),
        },
    }
}

/// 检查并自动安装 manifest.deps 声明的系统依赖。
/// 在任何文件写入之前执行：失败即中止安装，不产生半成品。
/// BIANWRT_AUTO_DEPS=0 可关闭自动安装（退化为只校验并报错）。
async fn ensure_deps(m: &Manifest) -> Result<serde_json::Value, String> {
    let Some(d) = &m.deps else {
        return Ok(json!({
            "manager": "none", "requested": [], "present": [],
            "installed": [], "commands": [], "auto": true
        }));
    };
    let mgr = detect_manager();
    let manager = match mgr {
        DepManager::Apt => "apt-get",
        DepManager::Opkg => "opkg",
        DepManager::Apk => "apk",
        DepManager::None => "none",
    };
    // apt 列表在 apk 系统上同样按包名使用；opkg 系统用 opkg 列表
    let wants: Vec<String> = match mgr {
        DepManager::Opkg => d.opkg.clone(),
        _ => d.apt.clone(),
    };
    if !wants.is_empty() && mgr == DepManager::None {
        return Err(format!(
            "声明了依赖包 {:?}，但本机未检测到 apt-get / opkg / apk，无法自动安装",
            wants
        ));
    }
    let auto = std::env::var("BIANWRT_AUTO_DEPS")
        .map(|v| v != "0" && v != "false")
        .unwrap_or(true);
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for p in &wants {
        if pkg_installed(mgr, p).await {
            present.push(p.clone());
        } else {
            missing.push(p.clone());
        }
    }
    let mut installed: Vec<String> = Vec::new();
    if !missing.is_empty() {
        if !auto {
            return Err(format!(
                "缺少依赖包: {}。自动安装已关闭（BIANWRT_AUTO_DEPS=0），请先手动安装",
                missing.join(", ")
            ));
        }
        let out = match mgr {
            DepManager::Apt => {
                let mut a = vec!["install", "-y", "--no-install-recommends"];
                a.extend(missing.iter().map(|s| s.as_str()));
                pkg_install("apt-get", &a, 300).await
            }
            DepManager::Opkg => {
                let mut a = vec!["install"];
                a.extend(missing.iter().map(|s| s.as_str()));
                pkg_install("opkg", &a, 300).await
            }
            DepManager::Apk => {
                let mut a = vec!["add", "--no-cache"];
                a.extend(missing.iter().map(|s| s.as_str()));
                pkg_install("apk", &a, 300).await
            }
            DepManager::None => unreachable!(),
        };
        if !out.ok {
            let tail = cmd::tail_lines(&format!("{}\n{}", out.stdout, out.stderr), 8).join("\n");
            return Err(format!("自动安装依赖失败: {}", tail));
        }
        for p in &missing {
            if !pkg_installed(mgr, p).await {
                return Err(format!("依赖包 {} 安装后仍不可用", p));
            }
        }
        installed = missing.clone();
    }
    // 命令级校验：安装完必须能在 PATH 找到
    let mut missing_cmd = Vec::new();
    for c in &d.commands {
        if !cmd::which(c) {
            missing_cmd.push(c.clone());
        }
    }
    if !missing_cmd.is_empty() {
        return Err(format!("依赖命令仍不可用: {}", missing_cmd.join(", ")));
    }
    Ok(json!({
        "manager": manager,
        "requested": wants,
        "present": present,
        "installed": installed,
        "commands": d.commands,
        "auto": auto,
    }))
}

// ── 文件系统写入与回滚 ──────────────────────────────────────────────

struct FsBackup {
    path: String,
    original: Option<Vec<u8>>,
}

/// 把 rootfs/ 前缀的条目铺到真实根目录 /，同时记录每个被写文件的原内容用于回滚。
fn install_rootfs(
    entries: &[PkgEntry],
    conffiles: &[String],
    backups: &mut Vec<FsBackup>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let cf: HashSet<&str> = conffiles.iter().map(|s| s.trim_start_matches('/')).collect();
    let mut installed = Vec::new();
    let mut kept = Vec::new();
    for x in entries {
        let rel_str = x.rel.to_string_lossy();
        let Some(rel) = rel_str.strip_prefix("rootfs/") else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        if !cmd::safe_rel(rel, 512) {
            return Err(format!("rootfs 路径非法: {}", rel));
        }
        let target = Path::new("/").join(rel);
        if x.is_dir {
            std::fs::create_dir_all(&target).map_err(|e| format!("创建目录 {} 失败: {}", rel, e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        let path = format!("/{}", rel);
        if cf.contains(rel) && target.exists() {
            kept.push(path.clone());
            installed.push(path);
            continue; // 用户配置：升级时不覆盖
        }
        backups.push(FsBackup {
            path: path.clone(),
            original: std::fs::read(&target).ok(),
        });
        std::fs::write(&target, &x.content).map_err(|e| format!("写入 /{} 失败: {}", rel, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if x.mode & 0o111 != 0 { x.mode } else { x.mode & !0o022 };
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
        }
        installed.push(path);
    }
    Ok((installed, kept))
}

fn rollback_rootfs(backups: &[FsBackup]) {
    for b in backups.iter().rev() {
        match &b.original {
            Some(data) => {
                let _ = std::fs::write(&b.path, data);
            }
            None => {
                let p = PathBuf::from(&b.path);
                let _ = std::fs::remove_file(&p);
                if let Some(parent) = p.parent() {
                    cleanup_empty_dirs(parent);
                }
            }
        }
    }
}

fn extract_plugin_files(entries: &[PkgEntry], dest: &Path) -> Result<(), String> {
    for x in entries {
        if x.rel.to_string_lossy().starts_with("rootfs/") || x.rel.as_os_str().is_empty() {
            continue;
        }
        let rel = x.rel.to_string_lossy();
        if !cmd::safe_rel(&rel, 512) {
            return Err(format!("包内路径非法: {}", rel));
        }
        let target = dest.join(&x.rel);
        if x.is_dir {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, &x.content).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(x.mode));
            }
        }
    }
    Ok(())
}

fn file_list_path(state: &AppState, name: &str) -> PathBuf {
    state.data_dir.join("plugin-files").join(format!("{}.list", name))
}

fn save_file_list(state: &AppState, name: &str, paths: &[String]) -> Result<(), String> {
    let p = file_list_path(state, name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, paths.join("\n")).map_err(|e| e.to_string())
}

fn load_file_list(state: &AppState, name: &str) -> Vec<String> {
    std::fs::read_to_string(file_list_path(state, name))
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn cleanup_empty_dirs(path: &Path) {
    const STOP: &[&str] = &[
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt",
        "/var", "/srv", "/home", "/root", "/boot", "/run", "/tmp",
    ];
    let mut cur = path.to_path_buf();
    for _ in 0..4 {
        let cur_str = cur.to_string_lossy().to_string();
        if STOP.contains(&cur_str.as_str()) || cur == Path::new("/") {
            return;
        }
        if std::fs::remove_dir(&cur).is_err() {
            return;
        }
        cur = match cur.parent() {
            Some(p) => p.to_path_buf(),
            None => return,
        };
    }
}

/// 升级成功后清理非当前版本目录（current 符号链接保留）。
fn prune_versions(base: &Path, keep: &str) {
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n == keep || n == "current" {
                continue;
            }
            let p = e.path();
            if p.is_symlink() || p.is_file() {
                let _ = std::fs::remove_file(&p);
            } else if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
}

struct Rollback {
    name: String,
    plugin_base: PathBuf,
    staging: PathBuf,
    version_dir: PathBuf,
    old_version_backup: Option<PathBuf>,
    current: PathBuf,
    old_current: Option<String>,
    unit_path: PathBuf,
    unit_tmp: PathBuf,
    old_unit: Option<Vec<u8>>,
    conf_manifest: PathBuf,
    old_conf_manifest: Option<Vec<u8>>,
    file_list: PathBuf,
    old_file_list: Option<Vec<u8>>,
    old_sha: Option<String>,
    old_signed: Option<String>,
    old_meta: Option<String>,
    meta_key: String,
    fs_backups: Vec<FsBackup>,
}

fn rollback(state: &AppState, rb: &Rollback) {
    rollback_rootfs(&rb.fs_backups);
    let _ = std::fs::remove_dir_all(&rb.staging);
    let _ = std::fs::remove_dir_all(&rb.version_dir);
    if let Some(b) = &rb.old_version_backup {
        let _ = std::fs::rename(b, &rb.version_dir);
    }
    let _ = std::fs::remove_file(&rb.current);
    if let Some(old) = &rb.old_current {
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(old, &rb.current);
        }
    } else if rb.old_version_backup.is_none() {
        // 全新安装失败：清掉可能已创建的目录或占位文件
        let _ = std::fs::remove_file(&rb.plugin_base);
        let _ = std::fs::remove_dir_all(&rb.plugin_base);
    }
    match &rb.old_unit {
        Some(d) => {
            let _ = std::fs::write(&rb.unit_path, d);
        }
        None => {
            let _ = std::fs::remove_file(&rb.unit_path);
        }
    }
    let _ = std::fs::remove_file(&rb.unit_tmp);
    match &rb.old_conf_manifest {
        Some(d) => {
            let _ = std::fs::write(&rb.conf_manifest, d);
        }
        None => {
            let _ = std::fs::remove_file(&rb.conf_manifest);
        }
    }
    match &rb.old_file_list {
        Some(d) => {
            let _ = std::fs::write(&rb.file_list, d);
        }
        None => {
            let _ = std::fs::remove_file(&rb.file_list);
        }
    }
    let db = state.db.lock().unwrap();
    for (k, old) in [
        (format!("plugin_sha.{}", rb.name), &rb.old_sha),
        (format!("plugin_signed.{}", rb.name), &rb.old_signed),
        (rb.meta_key.clone(), &rb.old_meta),
    ] {
        match old {
            Some(v) => set_setting(&db, &k, v),
            None => {
                db.execute("DELETE FROM settings WHERE key=?1", [k]).ok();
            }
        }
    }
}

// ── 安装 ────────────────────────────────────────────────────────────

async fn read_upload(multipart: &mut Multipart) -> Result<Vec<u8>, String> {
    let mut bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        if field.name() != Some("file") {
            continue;
        }
        let mut f = field;
        let mut buf = Vec::new();
        loop {
            match f.chunk().await {
                Ok(Some(c)) => {
                    if buf.len() + c.len() > MAX_UPLOAD {
                        return Err(format!("上传包超过 {}MB 上限", MAX_UPLOAD / 1024 / 1024));
                    }
                    buf.extend_from_slice(&c);
                }
                Ok(None) => break,
                Err(e) => return Err(format!("读取上传失败: {}", e)),
            }
        }
        bytes = Some(buf);
    }
    bytes.ok_or_else(|| "未找到上传文件字段 file".into())
}

struct InstallOutcome {
    rootfs_files: usize,
    kept_conffiles: Vec<String>,
}

pub async fn install(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let _guard = state.plugin_lock.lock().await;

    // 1. 流式读取上传
    let bytes = match read_upload(&mut multipart).await {
        Ok(b) => b,
        Err(e) => return cmd::err(e),
    };
    // 2. 解析归档（含解压炸弹限制）
    let entries = match parse_archive(&bytes) {
        Ok(v) => v,
        Err(e) => return cmd::err(e),
    };
    let manifest_raw = entries
        .iter()
        .find(|x| x.rel == Path::new("manifest.toml"))
        .map(|x| x.content.clone());
    let Some(manifest_raw) = manifest_raw else {
        return cmd::err("包内缺少 manifest.toml");
    };
    let manifest: Manifest = match toml::from_str(&String::from_utf8_lossy(&manifest_raw)) {
        Ok(m) => m,
        Err(e) => return cmd::err(format!("manifest 解析失败: {}", e)),
    };
    // 3. 字段校验（注入 / 枚举 / 路径逃逸）
    if let Err(e) = validate_manifest(&manifest) {
        return cmd::err(format!("manifest 校验失败: {}", e));
    }
    // 4. 完整性与签名
    let has_sums = match verify_checksums(&entries) {
        Ok(v) => v,
        Err(e) => return cmd::err(e),
    };
    let signed = match verify_signature(&state, &entries, &manifest_raw).await {
        Ok(v) => v,
        Err(e) => return cmd::err(e),
    };
    let pkg_sha = hex::encode(Sha256::digest(&bytes));
    let name = manifest.package.name.clone();
    let version = manifest.package.version.clone();

    // 5. 依赖检测 + 自动安装（事务外，失败不写任何文件）
    let deps_info = match ensure_deps(&manifest).await {
        Ok(v) => v,
        Err(e) => return cmd::err(format!("依赖检查失败: {}", e)),
    };
    let deps_installed: Vec<String> = deps_info["installed"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    // 6. 版本策略：禁止静默降级
    let installed_version: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row("SELECT version FROM plugins WHERE name=?1", [name.as_str()], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    };
    if let Some(cur) = &installed_version {
        if cmp_version(&version, cur) == Ordering::Less {
            return cmd::err(format!(
                "不允许降级：当前 {}，待装 {}。如需降级请先卸载后重装。",
                cur, version
            ));
        }
    }

    // 7. 准备路径与旧状态快照
    let plugin_base = state.plugins_dir.join(&name);
    let version_dir = plugin_base.join(&version);
    let staging = plugin_base.join(format!(".staging-{}", cmd::stamp()));
    let current = plugin_base.join("current");
    let unit_path = PathBuf::from(format!("/etc/systemd/system/{}", unit_name(&name)));
    let unit_tmp = PathBuf::from(format!(
        "{}.tmp-{}",
        unit_path.display(),
        cmd::stamp()
    ));
    let conf_manifest = state.conf_dir.join(&name).join("manifest.toml");
    let file_list = file_list_path(&state, &name);
    let sha_key = format!("plugin_sha.{}", name);
    let signed_key = format!("plugin_signed.{}", name);
    let meta_k = meta_key(&name);

    let mut rb = Rollback {
        name: name.clone(),
        plugin_base: plugin_base.clone(),
        staging: staging.clone(),
        version_dir: version_dir.clone(),
        old_version_backup: None,
        current: current.clone(),
        old_current: std::fs::read_link(&current)
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
        unit_path: unit_path.clone(),
        unit_tmp: unit_tmp.clone(),
        old_unit: std::fs::read(&unit_path).ok(),
        conf_manifest: conf_manifest.clone(),
        old_conf_manifest: std::fs::read(&conf_manifest).ok(),
        file_list: file_list.clone(),
        old_file_list: std::fs::read(&file_list).ok(),
        old_sha: None,
        old_signed: None,
        old_meta: None,
        meta_key: meta_k.clone(),
        fs_backups: Vec::new(),
    };
    {
        let db = state.db.lock().unwrap();
        rb.old_sha = get_setting(&db, &sha_key);
        rb.old_signed = get_setting(&db, &signed_key);
        rb.old_meta = get_setting(&db, &meta_k);
    }

    // 8. 提交（任一步失败完整回滚）
    let outcome: Result<InstallOutcome, String> = async {
        let (rootfs_files, kept) =
            install_rootfs(&entries, &manifest.package.conffiles, &mut rb.fs_backups)?;

        // 插件目录先写 staging，同版本重装时把旧目录挪开以便回滚
        std::fs::create_dir_all(&plugin_base).map_err(|e| e.to_string())?;
        if version_dir.exists() {
            let backup = plugin_base.join(format!(".old-{}", cmd::stamp()));
            let _ = std::fs::remove_dir_all(&backup);
            std::fs::rename(&version_dir, &backup).map_err(|e| e.to_string())?;
            rb.old_version_backup = Some(backup);
        }
        let _ = std::fs::remove_dir_all(&staging);
        extract_plugin_files(&entries, &staging)?;
        std::fs::rename(&staging, &version_dir).map_err(|e| e.to_string())?;

        // systemd unit（先写同目录临时文件再原子 rename）
        let has_service = manifest.service.is_some();
        if let Some(svc) = &manifest.service {
            let exec_abs = if Path::new(&svc.exec).is_absolute() {
                PathBuf::from(&svc.exec)
            } else {
                version_dir.join(&svc.exec)
            };
            let mut argv = vec![sd_quote(&exec_abs.to_string_lossy())];
            for a in &svc.args {
                argv.push(sd_quote(a));
            }
            let user = if svc.user.is_empty() { "root" } else { &svc.user };
            let wd = if svc.workdir.is_empty() {
                version_dir.display().to_string()
            } else {
                svc.workdir.clone()
            };
            let unit = format!(
                "[Unit]\nDescription=bianwrt plugin: {} - {}\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={}\nWorkingDirectory={}\nUser={}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=multi-user.target\n",
                name,
                manifest.package.description.replace(['\n', '\r'], " "),
                argv.join(" "),
                wd,
                user
            );
            if let Some(parent) = unit_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&unit_tmp, &unit).map_err(|e| format!("写入 systemd unit 失败: {}", e))?;
            std::fs::rename(&unit_tmp, &unit_path).map_err(|e| e.to_string())?;
            let _ = systemctl(&["daemon-reload"]).await;
            let _ = systemctl(&["enable", "--now", &unit_name(&name)]).await;
        } else {
            // 无服务插件：若之前版本有 unit，移除
            let _ = systemctl(&["disable", "--now", &unit_name(&name)]).await;
            let _ = std::fs::remove_file(&unit_path);
            let _ = systemctl(&["daemon-reload"]).await;
        }

        // conf/manifest.toml（卸载时判定 conffiles 的依据）
        if let Some(parent) = conf_manifest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&conf_manifest, &manifest_raw).map_err(|e| e.to_string())?;

        // 文件清单 + 整包指纹 + 运行期元数据（代理端口 / 依赖安装记录）
        save_file_list(&state, &name, &rootfs_files)?;
        {
            let db = state.db.lock().unwrap();
            set_setting(&db, &sha_key, &pkg_sha);
            set_setting(&db, &signed_key, if signed { "1" } else { "0" });
        }
        let mut meta = json!({ "deps": deps_info.clone() });
        if let Some(w) = &manifest.web {
            meta["port"] = json!(w.port);
            meta["timeout"] = json!(w.timeout);
        }
        save_meta(&state, &name, &meta);

        // current 原子切换
        let tmp_link = plugin_base.join(format!(".current-{}", cmd::stamp()));
        let _ = std::fs::remove_file(&tmp_link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&version, &tmp_link).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp_link, &current).map_err(|e| e.to_string())?;

        // db 登记（最后一步，SQLite 本地操作）
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT OR REPLACE INTO plugins(name,version,web_mode,has_service,description,author,homepage,installed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime'))",
                rusqlite::params![
                    name,
                    version,
                    manifest.package.web_mode,
                    has_service as i64,
                    manifest.package.description,
                    manifest.package.author,
                    manifest.package.homepage
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        // 清理旧版本目录与 .old 备份
        prune_versions(&plugin_base, &version);
        Ok(InstallOutcome {
            rootfs_files: rootfs_files.len(),
            kept_conffiles: kept,
        })
    }
    .await;

    match outcome {
        Ok(o) => Json(json!({
            "ok": true, "name": name, "url": format!("/plugins/{}/", name),
            "rootfs_files": o.rootfs_files, "kept_conffiles": o.kept_conffiles,
            "sha256": pkg_sha, "signed": signed, "has_sha256sums": has_sums,
            "web_mode": manifest.package.web_mode,
            "proxy_port": manifest.web.as_ref().map(|w| w.port).unwrap_or(0),
            "deps": deps_info,
        }))
        .into_response(),
        Err(e) => {
            rollback(&state, &rb);
            let extra = if deps_installed.is_empty() {
                String::new()
            } else {
                format!(
                    "；注意：本次自动安装的依赖包 [{}] 不会随回滚卸载，如不再需要请手动清理",
                    deps_installed.join(", ")
                )
            };
            cmd::fail(format!("安装失败，已回滚：{}{}", e, extra))
        }
    }
}

// ── 服务操作 / 卸载 ─────────────────────────────────────────────────

pub async fn action(
    State(state): State<AppState>,
    axum::extract::Path((name, op)): axum::extract::Path<(String, String)>,
) -> Response {
    let _guard = state.plugin_lock.lock().await;
    let has_service: bool = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT has_service FROM plugins WHERE name=?1",
            [name.as_str()],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v == 1)
        .unwrap_or(false)
    };
    if !has_service {
        return cmd::err("该插件没有后台服务");
    }
    let unit = unit_name(&name);
    let ok = match op.as_str() {
        "start" => systemctl(&["start", &unit]).await,
        "stop" => systemctl(&["stop", &unit]).await,
        "restart" => systemctl(&["restart", &unit]).await,
        _ => false,
    };
    if ok {
        Json(json!({"ok": true})).into_response()
    } else {
        cmd::fail("systemctl 操作失败")
    }
}

pub async fn uninstall(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let _guard = state.plugin_lock.lock().await;
    let purge_config = q
        .get("purge_config")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    // conffiles：升级时保留，卸载时同样保留（若未勾选清除配置）
    let keep: HashSet<String> = if purge_config {
        HashSet::new()
    } else {
        std::fs::read_to_string(state.conf_dir.join(&name).join("manifest.toml"))
            .ok()
            .and_then(|s| toml::from_str::<Manifest>(&s).ok())
            .map(|m| {
                m.package
                    .conffiles
                    .iter()
                    .map(|c| format!("/{}", c.trim_start_matches('/')))
                    .collect()
            })
            .unwrap_or_default()
    };

    let unit = unit_name(&name);
    let _ = systemctl(&["disable", "--now", &unit]).await;
    let _ = std::fs::remove_file(format!("/etc/systemd/system/{}", unit));
    let _ = systemctl(&["daemon-reload"]).await;

    let mut removed = 0usize;
    let mut kept_conffiles: Vec<String> = Vec::new();
    for f in load_file_list(&state, &name) {
        if keep.contains(&f) {
            kept_conffiles.push(f);
            continue;
        }
        let p = PathBuf::from(&f);
        if p == Path::new("/") || p.components().count() < 2 {
            continue;
        }
        if p.is_file() {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
            if let Some(parent) = p.parent() {
                cleanup_empty_dirs(parent);
            }
        }
    }
    let _ = std::fs::remove_file(file_list_path(&state, &name));
    let _ = std::fs::remove_dir_all(state.plugins_dir.join(&name));
    if purge_config {
        let _ = std::fs::remove_dir_all(state.conf_dir.join(&name));
    }
    {
        let db = state.db.lock().unwrap();
        db.execute("DELETE FROM plugins WHERE name=?1", [name.as_str()]).ok();
        db.execute(
            "DELETE FROM settings WHERE key IN (?1, ?2, ?3)",
            rusqlite::params![
                format!("plugin_sha.{}", name),
                format!("plugin_signed.{}", name),
                meta_key(&name)
            ],
        )
        .ok();
    }
    Json(json!({
        "ok": true, "removed_files": removed,
        "kept_conffiles": kept_conffiles, "config_kept": !purge_config
    }))
    .into_response()
}

// ── 插件页面入口：静态托管 / 反向代理 ──────────────────────────────

fn valid_plugin_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 统一入口：/plugins/<name>[/...]，按 web_mode 分派到静态文件服务或反向代理。
/// 所有 HTTP 方法都经过 gate 鉴权（gate 对 OPTIONS 预检放行，见 gate()）。
pub async fn plugin_entry(State(state): State<AppState>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let Some(all) = path.strip_prefix("/plugins/") else {
        return (StatusCode::NOT_FOUND, "插件路径不存在").into_response();
    };
    let (name, rest0) = match all.split_once('/') {
        Some((n, r)) => (n.to_string(), r.to_string()),
        None => (all.to_string(), String::new()),
    };
    if !valid_plugin_name(&name) {
        return cmd::err("非法插件名");
    }
    // 票据段仅用于鉴权，不属于插件自身路径；转发 / 静态查找前剥离
    let rest = strip_ticket_prefix(&rest0).to_string();
    let ticket = ticket_of_path(&path).map(|(_, t)| t);
    let web_mode: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT web_mode FROM plugins WHERE name=?1",
            [&name],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    match web_mode.as_deref() {
        Some("proxy") => proxy_to_plugin(&state, &name, &rest, req, ticket.as_deref()).await,
        Some("static") => {
            if req.method() == Method::OPTIONS {
                return cors_preflight();
            }
            if req.method() != Method::GET && req.method() != Method::HEAD {
                return (StatusCode::METHOD_NOT_ALLOWED, "静态插件仅支持 GET").into_response();
            }
            serve_plugin_file(&state, &name, &rest)
        }
        Some("none") => {
            (StatusCode::NOT_FOUND, "该插件为无页面模式（web_mode=none）").into_response()
        }
        _ => (StatusCode::NOT_FOUND, "插件不存在").into_response(),
    }
}

// hop-by-hop 与敏感头：一律不转发给插件进程 / 不回传浏览器
const SKIP_HEADERS: [&str; 12] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "authorization",
    "cookie",
];

fn cors_headers(builder: axum::http::response::Builder) -> axum::http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, POST, PUT, PATCH, DELETE, OPTIONS",
        )
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Content-Type, X-Requested-With, X-Bianwrt-Plugin",
        )
        .header(header::ACCESS_CONTROL_MAX_AGE, "600")
}

fn cors_preflight() -> Response {
    cors_headers(axum::http::Response::builder().status(StatusCode::NO_CONTENT))
        .body(Body::empty())
        .unwrap()
}

/// 插件自带 HTTP 服务的反向代理：仅连 127.0.0.1:<port>，剥离会话 token，
/// 过滤 hop-by-hop / Set-Cookie，重写相对 Location，解码 chunked，限制大小与超时。
async fn proxy_to_plugin(
    state: &AppState,
    name: &str,
    rest: &str,
    req: Request,
    ticket: Option<&str>,
) -> Response {
    let meta = load_meta(state, name);
    let port = meta.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    if port == 0 {
        return (StatusCode::BAD_GATEWAY, "插件未配置 [web].port 代理端口").into_response();
    }
    let timeout = meta
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(15)
        .clamp(1, 120);
    // 重定向等需要回落到插件页面时，必须保留票据前缀，否则后续请求会 401
    let base = match ticket {
        Some(t) => format!("/plugins/{}/_bw/{}", name, t),
        None => format!("/plugins/{}", name),
    };
    if req.method() == Method::OPTIONS {
        return cors_preflight();
    }
    if rest.contains("..") || rest.starts_with('/') {
        return cmd::err("非法资源路径");
    }

    // 目标 URI：去掉 /plugins/<name> 前缀，并剔除 gate 用的 token 参数（绝不下发给插件）
    let uri = req.uri().clone();
    let qs: Vec<&str> = uri
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|kv| !kv.starts_with("token=") && !kv.is_empty())
        .collect();
    let mut target = format!("/{}", rest);
    if !qs.is_empty() {
        target.push('?');
        target.push_str(&qs.join("&"));
    }

    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_PROXY_BODY).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "请求体超过 32MB 上限").into_response(),
    };

    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
        method.as_str(),
        target,
        port
    );
    head.push_str(&format!("X-Bianwrt-Plugin: {}\r\n", name));
    head.push_str("X-Forwarded-Proto: http\r\n");
    for (k, v) in headers.iter() {
        let kl = k.as_str().to_ascii_lowercase();
        if SKIP_HEADERS.contains(&kl.as_str()) {
            continue;
        }
        if let Ok(s) = v.to_str() {
            head.push_str(&format!("{}: {}\r\n", k.as_str(), s));
        }
    }
    if !body_bytes.is_empty() {
        head.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
    }
    head.push_str("\r\n");

    let io = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| e.to_string())?;
        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        if !body_bytes.is_empty() {
            stream
                .write_all(&body_bytes)
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > MAX_PROXY_BODY {
                return Err("插件响应超过 32MB 上限".to_string());
            }
        }
        Ok::<Vec<u8>, String>(buf)
    };
    let raw = match tokio::time::timeout(Duration::from_secs(timeout), io).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            return (StatusCode::BAD_GATEWAY, format!("插件服务请求失败: {}", e)).into_response()
        }
        Err(_) => return (StatusCode::GATEWAY_TIMEOUT, "插件服务响应超时").into_response(),
    };

    let Some(sep) = find_sub(&raw, b"\r\n\r\n") else {
        return (StatusCode::BAD_GATEWAY, "插件返回了非法 HTTP 响应").into_response();
    };
    let head_str = String::from_utf8_lossy(&raw[..sep]);
    let mut lines = head_str.split("\r\n");
    let code = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(502);
    let mut resp_headers: Vec<(String, String)> = Vec::new();
    let mut chunked = false;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let k = k.trim().to_string();
        let v = v.trim().to_string();
        if k.eq_ignore_ascii_case("transfer-encoding")
            && v.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
        resp_headers.push((k, v));
    }
    let mut body = raw[sep + 4..].to_vec();
    if chunked {
        match decode_chunked(&body) {
            Ok(b) => body = b,
            Err(e) => {
                return (StatusCode::BAD_GATEWAY, format!("chunked 解码失败: {}", e))
                    .into_response()
            }
        }
    }
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = axum::http::Response::builder().status(status);
    for (k, v) in &resp_headers {
        let kl = k.to_ascii_lowercase();
        // Set-Cookie 会覆盖 gate 下发的 bwpt 会话 cookie，一律丢弃；
        // hop-by-hop 与 Content-Length 由代理自己重建。
        if kl == "set-cookie" || kl == "content-length" || SKIP_HEADERS.contains(&kl.as_str()) {
            continue;
        }
        let val = if kl == "location" {
            rewrite_location(v, &base, port)
        } else {
            v.clone()
        };
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(&val),
        ) {
            builder = builder.header(hn, hv);
        }
    }
    builder = cors_headers(builder)
        .header(header::CONTENT_LENGTH, body.len())
        .header("X-Bianwrt-Proxy", name);
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| (StatusCode::BAD_GATEWAY, "代理响应构造失败").into_response())
}

fn rewrite_location(loc: &str, base: &str, port: u16) -> String {
    for host in ["127.0.0.1", "localhost"] {
        let root = format!("http://{}:{}", host, port);
        if loc == root {
            return format!("{}/", base);
        }
        let slash = format!("{}/", root);
        if let Some(tail) = loc.strip_prefix(&slash) {
            return format!("{}/{}", base, tail);
        }
    }
    if loc.starts_with('/') && !loc.starts_with("/plugins/") {
        return format!("{}{}", base, loc);
    }
    loc.to_string()
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn decode_chunked(mut d: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let pos = find_sub(d, b"\r\n").ok_or("chunked 缺少尺寸行")?;
        let line = std::str::from_utf8(&d[..pos]).map_err(|_| "chunked 尺寸行非法")?;
        let hex = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16).map_err(|_| "chunked 尺寸非法")?;
        d = &d[pos + 2..];
        if size == 0 {
            break;
        }
        if d.len() < size + 2 {
            return Err("chunked 数据不完整".into());
        }
        out.extend_from_slice(&d[..size]);
        d = &d[size + 2..];
    }
    Ok(out)
}

fn file_response(content: Vec<u8>, ct: &'static str) -> Response {
    cors_headers(
        axum::http::Response::builder()
            .header(header::CONTENT_TYPE, ct)
            .header(header::CONTENT_LENGTH, content.len()),
    )
    .body(Body::from(content))
    .unwrap()
}

fn serve_plugin_file(state: &AppState, name: &str, rest: &str) -> Response {
    if !valid_plugin_name(name) {
        return cmd::err("非法插件名");
    }
    let rel = if rest.is_empty() { "index.html" } else { rest };
    if !cmd::safe_rel(rel, 512) {
        return cmd::err("非法资源路径");
    }
    let base = state.plugins_dir.join(name).join("current").join("ui");
    let target = base.join(rel);
    if let Ok(content) = std::fs::read(&target) {
        return file_response(content, mime_of(rel));
    }
    // SPA fallback 只用于导航类请求；带静态资源扩展名的缺失文件返回真 404
    let ext = rel.rsplit('.').next().unwrap_or("");
    let is_asset = !ext.is_empty() && ext != "html" && ext.len() <= 8;
    if !is_asset {
        if let Ok(content) = std::fs::read(base.join("index.html")) {
            return file_response(content, "text/html; charset=utf-8");
        }
    }
    (
        StatusCode::NOT_FOUND,
        format!("插件 {} 未提供页面或资源 {}", name, rel),
    )
        .into_response()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "eot" => "application/vnd.ms-fontobject",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}
