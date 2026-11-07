//! Network → WireGuard：接口配置、密钥生成、启停与开机自启、状态。

use axum::{extract::{Path, State}, response::Response, Json};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

use crate::{cmd, state::AppState};

const WG_DIR: &str = "/etc/wireguard";

fn conf_path(name: &str) -> Option<PathBuf> {
    if !cmd::safe(name, 64) || name.contains("..") {
        return None;
    }
    Some(PathBuf::from(WG_DIR).join(format!("{}.conf", name)))
}

fn mask_private(content: &str) -> String {
    content
        .lines()
        .map(|l| {
            let t = l.trim().to_lowercase();
            if t.starts_with("privatekey") || t.starts_with("presharedkey") {
                let key = l.split_once('=').map(|x| x.0.trim()).unwrap_or("PrivateKey");
                format!("{} = <hidden>", key)
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn status() -> Response {
    if !cmd::which("wg") {
        return cmd::ok(json!({
            "available": false,
            "interfaces": [],
            "msg": "未安装 WireGuard 工具：apt install wireguard-tools"
        }));
    }
    let out = cmd::run("wg", &["show", "all", "dump"], 8).await;
    if !out.ok {
        return cmd::ok(json!({
            "available": false,
            "interfaces": [],
            "msg": format!("wg show 失败（需要 root）：{}", out.stderr.trim())
        }));
    }
    let mut ifaces: Map<String, Value> = Map::new();
    for line in out.stdout.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 || f[0].is_empty() {
            continue;
        }
        let name = f[0].to_string();
        if f.len() <= 5 {
            ifaces.insert(
                name.clone(),
                json!({
                    "name": name,
                    "public_key": f.get(2).unwrap_or(&""),
                    "listen_port": f.get(3).and_then(|x| x.parse::<u16>().ok()),
                    "fwmark": f.get(4).unwrap_or(&""),
                    "peers": [],
                }),
            );
        } else if let Some(iface) = ifaces.get_mut(&name).and_then(|v| v.as_object_mut()) {
            let peers = iface
                .entry("peers")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap();
            peers.push(json!({
                "public_key": f[1],
                "endpoint": f.get(3).unwrap_or(&"(none)"),
                "allowed_ips": f.get(4).unwrap_or(&""),
                "latest_handshake": f.get(5).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0),
                "rx": f.get(6).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0),
                "tx": f.get(7).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0),
                "keepalive": f.get(8).unwrap_or(&"0"),
            }));
        }
    }
    let items: Vec<Value> = ifaces.into_values().collect();
    cmd::ok(json!({ "available": true, "interfaces": items }))
}

pub async fn interfaces(State(_state): State<AppState>) -> Response {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(WG_DIR) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.ends_with(".conf") {
                continue;
            }
            let name = n.trim_end_matches(".conf").to_string();
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            let en = cmd::run("systemctl", &["is-enabled", &format!("wg-quick@{}", name)], 5).await;
            items.push(json!({
                "name": name,
                "size": content.len(),
                "enabled": en.stdout.trim() == "enabled",
                "content": mask_private(&content),
            }));
        }
    }
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    cmd::ok(json!({ "items": items, "dir": WG_DIR, "available": cmd::which("wg-quick") }))
}

#[derive(Deserialize)]
pub struct SaveReq {
    pub name: String,
    pub config: String,
}

pub async fn put(Json(req): Json<SaveReq>) -> Response {
    let Some(path) = conf_path(&req.name) else {
        return cmd::err("接口名非法（仅允许字母、数字、.-_）");
    };
    if req.config.len() > 65536 || req.config.contains('\0') {
        return cmd::err("配置内容过长或含非法字符");
    }
    if !req.config.contains("[Interface]") {
        return cmd::err("配置缺少 [Interface] 段");
    }
    if std::fs::create_dir_all(WG_DIR).is_err() {
        return cmd::fail("无法创建 /etc/wireguard（需要 root）");
    }
    if std::fs::write(&path, &req.config).is_err() {
        return cmd::fail("写入配置失败（需要 root）");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    cmd::ok(json!({ "ok": true, "path": path.display().to_string() }))
}

pub async fn del(Path(name): Path<String>) -> Response {
    let Some(path) = conf_path(&name) else {
        return cmd::err("接口名非法");
    };
    match std::fs::remove_file(&path) {
        Ok(_) => cmd::ok(json!({ "ok": true })),
        Err(e) => cmd::fail(format!("删除失败：{}", e)),
    }
}

#[derive(Deserialize)]
pub struct ActionReq {
    pub name: String,
    pub op: String,
}

pub async fn action(Json(req): Json<ActionReq>) -> Response {
    if !cmd::safe(&req.name, 64) {
        return cmd::err("接口名非法");
    }
    let args: Vec<String> = match req.op.as_str() {
        "up" => vec!["wg-quick".into(), "up".into(), req.name.clone()],
        "down" => vec!["wg-quick".into(), "down".into(), req.name.clone()],
        "enable" => vec!["systemctl".into(), "enable".into(), format!("wg-quick@{}", req.name)],
        "disable" => vec!["systemctl".into(), "disable".into(), format!("wg-quick@{}", req.name)],
        _ => return cmd::err("操作须为 up / down / enable / disable"),
    };
    let program = args[0].clone();
    let refs: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
    let out = cmd::run(&program, &refs, 30).await;
    if !out.ok {
        return cmd::fail(format!(
            "{} 失败：{}{}",
            req.op,
            out.stderr.trim(),
            if out.stdout.is_empty() { String::new() } else { format!("\n{}", out.stdout.trim()) }
        ));
    }
    cmd::ok(json!({ "ok": true, "msg": format!("wg-quick@{} {} 成功", req.name, req.op) }))
}

pub async fn keygen() -> Response {
    if !cmd::which("wg") {
        return cmd::fail("未安装 wireguard-tools：apt install wireguard-tools");
    }
    let priv_out = cmd::run("wg", &["genkey"], 8).await;
    if !priv_out.ok {
        return cmd::fail(format!("生成私钥失败：{}", priv_out.stderr.trim()));
    }
    let private = priv_out.stdout.trim().to_string();
    let pub_out = cmd::run_stdin("wg", &["pubkey"], private.as_bytes(), 8).await;
    let psk_out = cmd::run("wg", &["genpsk"], 8).await;
    cmd::ok(json!({
        "private": private,
        "public": pub_out.stdout.trim(),
        "preshared": psk_out.stdout.trim(),
        "error": if pub_out.ok { "" } else { pub_out.stderr.trim() },
    }))
}
