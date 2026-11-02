use axum::extract::multipart::Multipart;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tar::Archive;

use crate::state::AppState;

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
    /// rootfs 内的用户配置文件路径（相对 rootfs/），升级时若已存在则保留不覆盖
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
}

#[derive(Deserialize, Clone)]
pub struct Manifest {
    pub package: PackageMeta,
    pub service: Option<ServiceMeta>,
    pub deps: Option<DepsMeta>,
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
        items.push(json!({
            "name": name, "version": version, "web_mode": web_mode,
            "has_service": has_service == 1, "service_exists": exists, "active": active,
            "description": description, "author": author, "homepage": homepage,
            "installed_at": installed_at,
            "url": format!("/plugins/{}/", name),
        }));
    }
    Json(json!({"items": items})).into_response()
}

/// 包内一个条目：(相对路径, 内容, 是否目录, unix 权限)
struct PkgEntry {
    rel: PathBuf,
    content: Vec<u8>,
    is_dir: bool,
    mode: u32,
}

/// 解析 tar.gz 到内存（带路径穿越防护 + 兼容单一顶层目录的归档）
fn parse_archive(bytes: &[u8]) -> Result<Vec<PkgEntry>, String> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
    let entries = archive.entries().map_err(|e| format!("包读取失败: {}", e))?;
    let mut all: Vec<PkgEntry> = Vec::new();
    let mut roots: Vec<Option<String>> = Vec::new();
    for e in entries {
        let mut e = e.map_err(|e| format!("包损坏: {}", e))?;
        let path = e.path().map_err(|e| e.to_string())?.to_path_buf();
        let p = path.to_string_lossy();
        if path.is_absolute() || p.contains("..") {
            return Err(format!("包内存在非法路径: {}", p));
        }
        let comps: Vec<_> = path.components().collect();
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
    // 兼容 "demo-hello/manifest.toml" 这种带顶层目录的包：所有条目同根且 manifest 不在顶层时剥掉
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

/// 把 rootfs/ 前缀的条目铺到真实根目录 /，返回实际安装的绝对路径列表。
/// conffiles 中声明的路径若已存在则保留用户版本（不覆盖），但同样记入清单。
fn install_rootfs(
    entries: &[PkgEntry],
    conffiles: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let cf: std::collections::HashSet<&str> =
        conffiles.iter().map(|s| s.trim_start_matches('/')).collect();
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
        let target = Path::new("/").join(rel);
        if x.is_dir {
            std::fs::create_dir_all(&target).map_err(|e| format!("创建目录 {} 失败: {}", rel, e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        let is_conffile = cf.contains(rel);
        if is_conffile && target.exists() {
            kept.push(format!("/{}", rel));
        } else {
            std::fs::write(&target, &x.content).map_err(|e| format!("写入 /{} 失败: {}", rel, e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if x.mode & 0o111 != 0 { x.mode } else { x.mode & !0o022 };
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
            }
        }
        installed.push(format!("/{}", rel));
    }
    Ok((installed, kept))
}

/// 把非 rootfs 条目（manifest/ui/bin 等）写入插件版本目录
fn extract_plugin_files(entries: &[PkgEntry], dest: &Path) -> Result<(), String> {
    for x in entries {
        if x.rel.to_string_lossy().starts_with("rootfs/") || x.rel.as_os_str().is_empty() {
            continue;
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
    state.home.join("plugin-files").join(format!("{}.list", name))
}

fn save_file_list(state: &AppState, name: &str, paths: &[String]) {
    let p = file_list_path(state, name);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&p, paths.join("\n"));
}

fn load_file_list(state: &AppState, name: &str) -> Vec<String> {
    std::fs::read_to_string(file_list_path(state, name))
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

/// 空目录向上清理（遇到系统一级目录或非空目录即停）
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
            return; // 非空或无权限即停止
        }
        cur = match cur.parent() {
            Some(p) => p.to_path_buf(),
            None => return,
        };
    }
}

pub async fn install(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut bytes: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            match field.bytes().await {
                Ok(b) => bytes = Some(b.to_vec()),
                Err(e) => return (StatusCode::BAD_REQUEST, format!("读取上传失败: {}", e)).into_response(),
            }
        }
    }
    let Some(bytes) = bytes else {
        return (StatusCode::BAD_REQUEST, "未找到上传文件字段 file").into_response();
    };
    // 1. 解析归档
    let entries = match parse_archive(&bytes) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let manifest_raw = entries
        .iter()
        .find(|x| x.rel == Path::new("manifest.toml"))
        .map(|x| x.content.clone());
    let Some(manifest_raw) = manifest_raw else {
        return (StatusCode::BAD_REQUEST, "包内缺少 manifest.toml").into_response();
    };
    let manifest: Manifest = match toml::from_str(&String::from_utf8_lossy(&manifest_raw)) {
        Ok(m) => m,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("manifest 解析失败: {}", e)).into_response(),
    };
    let name = manifest.package.name.clone();
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || name.is_empty() {
        return (StatusCode::BAD_REQUEST, "插件名仅允许小写字母/数字/连字符").into_response();
    }
    // 2. apt 依赖校验
    if let Some(deps) = &manifest.deps {
        for p in &deps.apt {
            let out = tokio::process::Command::new("dpkg-query")
                .args(["-W", "-f=${db:Status-Abbrev}", p])
                .output()
                .await;
            let ok = out.map(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).trim().starts_with("ii")
            });
            if !matches!(ok, Ok(true)) {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("缺少依赖: {}，请先安装 (apt install {})", p, p),
                )
                    .into_response();
            }
        }
    }
    // 3. rootfs 铺到真实根目录（先执行，失败即中止且不落任何插件元数据）
    let (rootfs_files, kept_conffiles) = match install_rootfs(&entries, &manifest.package.conffiles) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    // 4. ui/manifest 等写入插件版本目录
    let version = manifest.package.version.clone();
    let final_dir = state.plugins_dir.join(&name).join(&version);
    let _ = std::fs::remove_dir_all(&final_dir);
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    if let Err(e) = extract_plugin_files(&entries, &final_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("插件目录写入失败: {}", e)).into_response();
    }
    // current 符号链接
    let current = state.plugins_dir.join(&name).join("current");
    let _ = std::fs::remove_file(&current);
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&version, &current);
    }
    // 5. 配置目录（类似 OpenWrt 的 /etc/config，卸载默认保留）
    let cfg_dir = state.etc_dir.join(&name);
    let _ = std::fs::create_dir_all(&cfg_dir);
    let _ = std::fs::write(
        cfg_dir.join("manifest.toml"),
        String::from_utf8_lossy(&manifest_raw).to_string(),
    );
    // 6. 文件清单（卸载清理依据）
    save_file_list(&state, &name, &rootfs_files);
    // 7. systemd unit
    let has_service = manifest.service.is_some();
    if let Some(svc) = &manifest.service {
        let exec_abs = if Path::new(&svc.exec).is_absolute() {
            PathBuf::from(&svc.exec)
        } else {
            final_dir.join(&svc.exec)
        };
        let args = svc
            .args
            .iter()
            .map(|a| format!("\"{}\"", a))
            .collect::<Vec<_>>()
            .join(" ");
        let user = if svc.user.is_empty() { "root" } else { &svc.user };
        let wd = if svc.workdir.is_empty() {
            final_dir.display().to_string()
        } else {
            svc.workdir.clone()
        };
        let unit = format!(
            "[Unit]\nDescription=bianwrt plugin: {} - {}\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={} {}\nWorkingDirectory={}\nUser={}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=multi-user.target\n",
            name,
            manifest.package.description.replace('\n', " "),
            exec_abs.display(),
            args,
            wd,
            user
        );
        let unit_path = format!("/etc/systemd/system/{}", unit_name(&name));
        if std::fs::write(&unit_path, unit).is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "写入 systemd unit 失败").into_response();
        }
        let _ = systemctl(&["daemon-reload"]).await;
        let _ = systemctl(&["enable", "--now", &unit_name(&name)]).await;
    }
    // 8. db 登记
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
        .ok();
    }
    Json(json!({
        "ok": true, "name": name, "url": format!("/plugins/{}/", name),
        "rootfs_files": rootfs_files.len(), "kept_conffiles": kept_conffiles,
    }))
    .into_response()
}

pub async fn action(
    State(state): State<AppState>,
    axum::extract::Path((name, op)): axum::extract::Path<(String, String)>,
) -> Response {
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
        return (StatusCode::BAD_REQUEST, "该插件没有后台服务").into_response();
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
        (StatusCode::INTERNAL_SERVER_ERROR, "systemctl 操作失败").into_response()
    }
}

pub async fn uninstall(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let purge_config = q.get("purge_config").map(|v| v == "1" || v == "true").unwrap_or(false);
    // 1. 停服务
    let unit = unit_name(&name);
    let _ = systemctl(&["disable", "--now", &unit]).await;
    let _ = std::fs::remove_file(format!("/etc/systemd/system/{}", unit));
    let _ = systemctl(&["daemon-reload"]).await;
    // 2. 按文件清单精确清理 rootfs 铺出去的文件
    let mut removed = 0usize;
    for f in load_file_list(&state, &name) {
        let p = PathBuf::from(&f);
        if p == Path::new("/") || p.components().count() < 2 {
            continue; // 不可能删根
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
    // 3. 插件目录（ui/manifest/版本）
    let _ = std::fs::remove_dir_all(state.plugins_dir.join(&name));
    // 4. 配置目录：默认保留，勾选才删
    let cfg_dir = state.etc_dir.join(&name);
    if purge_config {
        let _ = std::fs::remove_dir_all(&cfg_dir);
    }
    // 5. db 注销
    {
        let db = state.db.lock().unwrap();
        db.execute("DELETE FROM plugins WHERE name=?1", [name.as_str()]).ok();
    }
    Json(json!({"ok": true, "removed_files": removed, "config_kept": !purge_config})).into_response()
}

/// serve plugin static ui: /plugins/{name} or /plugins/{name}/  -> index.html
pub async fn plugin_static(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    serve_plugin_file(state, &name, "")
}

/// serve /plugins/{name}/{*rest}
pub async fn plugin_static_rest(
    State(state): State<AppState>,
    axum::extract::Path((name, rest)): axum::extract::Path<(String, String)>,
) -> Response {
    serve_plugin_file(state, &name, &rest)
}

fn serve_plugin_file(state: AppState, name: &str, rest: &str) -> Response {
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return (StatusCode::BAD_REQUEST, "非法插件名").into_response();
    }
    let base = state.plugins_dir.join(name).join("current").join("ui");
    let rel = if rest.is_empty() { "index.html".to_string() } else { rest.to_string() };
    let target = base.join(&rel);
    if let Ok(content) = std::fs::read(&target) {
        let mime = mime_of(&rel);
        return (
            [(axum::http::header::CONTENT_TYPE, mime)],
            content,
        )
            .into_response();
    }
    // SPA fallback
    if let Ok(content) = std::fs::read(base.join("index.html")) {
        return (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            content,
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, format!("插件 {} 未提供页面", name)).into_response()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}
