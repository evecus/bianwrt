//! System → Backup / Flash Firmware：配置备份、下载、上传、在线恢复。

use axum::{
    extract::{Multipart, Path, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use tar::{Archive, Builder, Header};

use crate::{cmd, state::AppState};

type Files = Vec<(String, Vec<u8>)>;

fn backups_dir(state: &AppState) -> PathBuf {
    state.work_dir.join("backups")
}

fn safe_name(name: &str) -> bool {
    (name.starts_with("bianwrt-") || name.starts_with("upload-"))
        && name.ends_with(".tar.gz")
        && cmd::safe(name, 128)
}

fn push_file(files: &mut Files, name: &str, path: &PathBuf) {
    if let Ok(b) = std::fs::read(path) {
        files.push((name.into(), b));
    }
}

/// 递归收集普通文件（跳过符号链接，例如插件的 current）。
fn collect_tree(root: &std::path::Path, prefix: &str, files: &mut Files) {
    fn walk(dir: &std::path::Path, rel: &str, prefix: &str, files: &mut Files) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !cmd::safe(&n, 128) {
                continue;
            }
            let r = if rel.is_empty() { n } else { format!("{}/{}", rel, n) };
            let p = e.path();
            let is_link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            if is_link {
                continue;
            }
            if p.is_dir() {
                walk(&p, &r, prefix, files);
            } else if p.is_file() {
                push_file(files, &format!("{}/{}", prefix, r), &p);
            }
        }
    }
    walk(root, "", prefix, files);
}

async fn collect(state: &AppState) -> Result<Files, String> {
    let mut files: Files = Vec::new();
    {
        let db = state.db.lock().unwrap();
        let _ = db.execute_batch("PRAGMA wal_checkpoint(FULL)");
    }
    let dbp = state.data_dir.join("bianwrt.db");
    files.push((
        "bianwrt.db".into(),
        std::fs::read(&dbp).map_err(|e| format!("读取数据库失败：{}", e))?,
    ));

    // conf/ 递归收集：包含插件的 <name>/manifest.toml（卸载时判定 conffiles 的依据）
    collect_tree(&state.conf_dir, "conf", &mut files);
    push_file(&mut files, "generated/bianwrt.nft", &state.data_dir.join("bianwrt.nft"));
    push_file(
        &mut files,
        "generated/dnsmasq-preview.conf",
        &state.data_dir.join("dnsmasq-preview.conf"),
    );
    push_file(
        &mut files,
        "etc/dnsmasq-bianwrt.conf",
        &PathBuf::from("/etc/dnsmasq.d/bianwrt.conf"),
    );
    push_file(
        &mut files,
        "etc/dnsmasq-bianwrt-filter.conf",
        &PathBuf::from("/etc/dnsmasq.d/bianwrt-filter.conf"),
    );
    push_file(&mut files, "generated/panel.nft", &PathBuf::from("/etc/bianwrt/panel.nft"));
    push_file(
        &mut files,
        "etc/hostapd.conf",
        &PathBuf::from("/etc/hostapd/hostapd.conf"),
    );

    let cr = cmd::run("crontab", &["-l"], 5).await;
    if cr.ok && !cr.stdout.is_empty() {
        files.push(("crontab.txt".into(), cr.stdout.into_bytes()));
    }

    if let Ok(rd) = std::fs::read_dir("/etc/wireguard") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".conf") && cmd::safe(&n, 128) {
                push_file(&mut files, &format!("wireguard/{}", n), &e.path());
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir("/etc/network/interfaces.d") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("bianwrt-") && n.ends_with(".conf") && cmd::safe(&n, 128) {
                push_file(&mut files, &format!("interfaces.d/{}", n), &e.path());
            }
        }
    }

    // 插件：安装目录（跳过 current 符号链接）、文件清单、systemd unit、rootfs 铺出的文件
    collect_tree(&state.plugins_dir, "plugins", &mut files);
    collect_tree(&state.data_dir.join("plugin-files"), "plugin-files", &mut files);
    if let Ok(rd) = std::fs::read_dir("/etc/systemd/system") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("bianwrt-plugin-") && n.ends_with(".service") && cmd::safe(&n, 128) {
                push_file(&mut files, &format!("systemd/{}", n), &e.path());
            }
        }
    }
    // 插件铺到系统根目录的文件（按清单收集，恢复时可还原）
    if let Ok(rd) = std::fs::read_dir(state.data_dir.join("plugin-files")) {
        for e in rd.flatten() {
            if !e.path().is_file() {
                continue;
            }
            if let Ok(list) = std::fs::read_to_string(e.path()) {
                for line in list.lines().filter(|l| !l.trim().is_empty()) {
                    let p = line.trim();
                    if p.starts_with('/') && !p.contains("..") {
                        push_file(&mut files, &format!("plugin-rootfs{}", p), &PathBuf::from(p));
                    }
                }
            }
        }
    }

    let manifest = json!({
        "format": "bianwrt-backup",
        "version": 1,
        "created": chrono::Local::now().to_rfc3339(),
        "hostname": sysinfo::System::host_name().unwrap_or_default(),
        "files": files.iter().map(|(n, _)| n).collect::<Vec<_>>(),
    });
    files.insert(0, ("manifest.json".into(), serde_json::to_vec_pretty(&manifest).unwrap()));
    Ok(files)
}

fn pack(files: &Files) -> Result<Vec<u8>, String> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut b = Builder::new(gz);
    for (name, data) in files {
        let mut h = Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o600);
        h.set_mtime(chrono::Utc::now().timestamp() as u64);
        h.set_cksum();
        b.append_data(&mut h, name, data.as_slice())
            .map_err(|e| format!("打包 {} 失败：{}", name, e))?;
    }
    let gz = b.into_inner().map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())
}

fn unpack(bytes: &[u8]) -> Result<Files, String> {
    let mut ar = Archive::new(GzDecoder::new(Cursor::new(bytes)));
    let mut out = Vec::new();
    for e in ar.entries().map_err(|e| e.to_string())? {
        let mut e = e.map_err(|e| e.to_string())?;
        let path = e.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
        if path.starts_with('/') || path.contains("..") || !e.header().entry_type().is_file() {
            continue;
        }
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        out.push((path, buf));
    }
    Ok(out)
}

fn prune(dir: &PathBuf, keep: usize) {
    let mut v: Vec<(u64, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                if m.is_file() {
                    v.push((
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                        e.path(),
                    ));
                }
            }
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in v.iter().skip(keep) {
        let _ = std::fs::remove_file(p);
    }
}

pub async fn create(State(state): State<AppState>) -> Response {
    let dir = backups_dir(&state);
    if std::fs::create_dir_all(&dir).is_err() {
        return cmd::fail("无法创建备份目录");
    }
    let files = match collect(&state).await {
        Ok(f) => f,
        Err(e) => return cmd::fail(e),
    };
    let bytes = match pack(&files) {
        Ok(b) => b,
        Err(e) => return cmd::fail(e),
    };
    let name = format!("bianwrt-{}.tar.gz", cmd::stamp());
    if std::fs::write(dir.join(&name), &bytes).is_err() {
        return cmd::fail("写入备份文件失败");
    }
    prune(&dir, 20);
    cmd::ok(json!({ "ok": true, "name": name, "size": bytes.len(), "files": files.len() }))
}

pub async fn list(State(state): State<AppState>) -> Response {
    let dir = backups_dir(&state);
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !safe_name(&n) {
                continue;
            }
            let m = e.metadata().ok();
            items.push(json!({
                "name": n,
                "size": m.as_ref().map(|m| m.len()).unwrap_or(0),
                "created": m.as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default())
                    .unwrap_or_default(),
            }));
        }
    }
    items.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    cmd::ok(json!({ "items": items, "dir": dir.display().to_string() }))
}

pub async fn download(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Response {
    if !safe_name(&name) {
        return cmd::err("备份文件名非法");
    }
    let bytes = match std::fs::read(backups_dir(&state).join(&name)) {
        Ok(b) => b,
        Err(_) => return cmd::fail("备份文件不存在"),
    };
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/gzip"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    (h, bytes).into_response()
}

pub async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let data = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return cmd::fail(format!("读取上传文件失败：{}", e)),
        };
        if data.len() > 64 * 1024 * 1024 {
            return cmd::err("备份文件超过 64MB 上限");
        }
        if data.len() < 2 || data[0] != 0x1f || data[1] != 0x8b {
            return cmd::err("文件不是有效的 gzip 备份");
        }
        if unpack(data.as_ref()).is_err() {
            return cmd::err("备份包无法解压或格式不正确");
        }
        let dir = backups_dir(&state);
        let _ = std::fs::create_dir_all(&dir);
        let name = format!("upload-{}.tar.gz", cmd::stamp());
        if std::fs::write(dir.join(&name), data.as_ref()).is_err() {
            return cmd::fail("保存备份失败");
        }
        return cmd::ok(json!({ "ok": true, "name": name, "size": data.len() }));
    }
    cmd::err("未收到 file 字段")
}

#[derive(Deserialize)]
pub struct RestoreReq {
    pub name: String,
}

pub async fn restore(
    State(state): State<AppState>,
    Json(req): Json<RestoreReq>,
) -> Response {
    if !safe_name(&req.name) {
        return cmd::err("备份文件名非法");
    }
    let bytes = match std::fs::read(backups_dir(&state).join(&req.name)) {
        Ok(b) => b,
        Err(_) => return cmd::fail("备份文件不存在"),
    };
    let files = match unpack(&bytes) {
        Ok(f) => f,
        Err(e) => return cmd::fail(format!("解压失败：{}", e)),
    };
    let get = |n: &str| files.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    let mf: Value = match get("manifest.json").and_then(|b| serde_json::from_slice(&b).ok()) {
        Some(v) => v,
        None => return cmd::err("备份包缺少 manifest.json"),
    };
    if mf["format"] != "bianwrt-backup" {
        return cmd::err("不是 bianwrt 配置备份");
    }
    let dbbytes = match get("bianwrt.db") {
        Some(b) => b,
        None => return cmd::err("备份包缺少 bianwrt.db"),
    };

    let tmp = state.data_dir.join(format!("restore-{}.db", cmd::stamp()));
    if std::fs::write(&tmp, &dbbytes).is_err() {
        return cmd::fail("写入临时数据库失败");
    }

    let mut restored: Vec<String> = Vec::new();
    {
        let mut db = state.db.lock().unwrap();
        let admin: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key='admin'", [], |r| r.get(0))
            .ok();
        let _ = db.execute_batch("PRAGMA wal_checkpoint(FULL)");
        let attach = db.execute(
            "ATTACH DATABASE ?1 AS restore",
            [tmp.to_string_lossy().to_string()],
        );
        if let Err(e) = attach {
            let _ = std::fs::remove_file(&tmp);
            return cmd::fail(format!("挂载备份数据库失败：{}", e));
        }
        let tables = [
            "settings", "fw_zones", "fw_rules", "fw_forwards", "dhcp_static", "plugins",
        ];
        let r: Result<(), rusqlite::Error> = (|| {
            let tx = db.transaction()?;
            for t in tables {
                tx.execute(&format!("DELETE FROM main.{}", t), [])?;
                tx.execute(
                    &format!("INSERT INTO main.{} SELECT * FROM restore.{}", t, t),
                    [],
                )?;
                restored.push(t.to_string());
            }
            tx.commit()
        })();
        if let Err(e) = r {
            let _ = db.execute_batch("DETACH DATABASE restore");
            let _ = std::fs::remove_file(&tmp);
            return cmd::fail(format!("恢复数据库失败（配置未改动）：{}", e));
        }
        // 保留当前管理员账号，避免恢复旧备份后把自己锁在门外
        if let Some(a) = admin {
            let _ = db.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES('admin',?1)",
                [a],
            );
        }
        let _ = db.execute_batch("DETACH DATABASE restore");
    }
    let _ = std::fs::remove_file(&tmp);

    // 恢复受管配置文件（仅白名单路径）
    for (name, data) in &files {
        let target: Option<PathBuf> = if let Some(n) = name.strip_prefix("conf/") {
            if cmd::safe_rel(n, 512) {
                Some(state.conf_dir.join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("generated/") {
            if cmd::safe(n, 128) {
                Some(state.data_dir.join(n))
            } else {
                None
            }
        } else if name == "etc/dnsmasq-bianwrt.conf" {
            Some(PathBuf::from("/etc/dnsmasq.d/bianwrt.conf"))
        } else if name == "etc/dnsmasq-bianwrt-filter.conf" {
            Some(PathBuf::from("/etc/dnsmasq.d/bianwrt-filter.conf"))
        } else if name == "etc/hostapd.conf" {
            Some(PathBuf::from("/etc/hostapd/hostapd.conf"))
        } else if let Some(n) = name.strip_prefix("wireguard/") {
            if cmd::safe(n, 128) {
                Some(PathBuf::from("/etc/wireguard").join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("interfaces.d/") {
            if cmd::safe(n, 128) {
                Some(PathBuf::from("/etc/network/interfaces.d").join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("plugins/") {
            if cmd::safe_rel(n, 512) {
                Some(state.plugins_dir.join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("plugin-files/") {
            if cmd::safe_rel(n, 128) {
                Some(state.data_dir.join("plugin-files").join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("systemd/") {
            if n.starts_with("bianwrt-plugin-") && n.ends_with(".service") && cmd::safe(n, 128) {
                Some(PathBuf::from("/etc/systemd/system").join(n))
            } else {
                None
            }
        } else if let Some(n) = name.strip_prefix("plugin-rootfs/") {
            if cmd::safe_rel(n, 512) {
                Some(PathBuf::from("/").join(n))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(p) = target {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if std::fs::write(&p, data).is_ok() {
                restored.push(name.clone());
            }
        }
    }
    if let Some(cron) = get("crontab.txt") {
        let p = state.data_dir.join("crontab-restore.tmp");
        if std::fs::write(&p, &cron).is_ok() {
            let ps = p.to_string_lossy().to_string();
            if cmd::run("crontab", &[&ps], 8).await.ok {
                restored.push("crontab".into());
            }
            let _ = std::fs::remove_file(&p);
        }
    }

    // 备份中跳过符号链接，按已恢复的 plugins 表重建 <name>/current -> <version>
    let plugin_rows: Vec<(String, String)> = {
        let db = state.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT name, version FROM plugins").unwrap();
        let v = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .flatten()
            .collect();
        v
    };
    for (n, ver) in plugin_rows {
        let base = state.plugins_dir.join(&n);
        if !base.join(&ver).is_dir() {
            continue;
        }
        let cur = base.join("current");
        let _ = std::fs::remove_file(&cur);
        #[cfg(unix)]
        if std::os::unix::fs::symlink(&ver, &cur).is_ok() {
            restored.push(format!("plugins/{}/current", n));
        }
    }
    // 旧格式备份（未打包 plugins/）恢复后，清掉 db 里文件已不存在的孤立插件记录，
    // 避免菜单/列表出现点不开的幽灵插件。
    {
        let db = state.db.lock().unwrap();
        let rows: Vec<String> = {
            let mut stmt = db.prepare("SELECT name FROM plugins").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .flatten()
                .collect()
        };
        for n in rows {
            if !state.plugins_dir.join(&n).is_dir() {
                db.execute("DELETE FROM plugins WHERE name=?1", [n]).ok();
            }
        }
    }
    if cmd::run("systemctl", &["daemon-reload"], 8).await.ok {
        restored.push("systemd/daemon-reload".into());
    }

    cmd::ok(json!({
        "ok": true,
        "created": mf["created"],
        "restored": restored,
        "msg": "配置已恢复；当前会话与管理员密码保持不变。防火墙/DHCP/DNS 请到对应页面重新『应用』使其生效。"
    }))
}

pub async fn delete(Path(name): Path<String>, State(state): State<AppState>) -> Response {
    if !safe_name(&name) {
        return cmd::err("备份文件名非法");
    }
    match std::fs::remove_file(backups_dir(&state).join(&name)) {
        Ok(_) => cmd::ok(json!({ "ok": true })),
        Err(e) => cmd::fail(format!("删除失败：{}", e)),
    }
}
