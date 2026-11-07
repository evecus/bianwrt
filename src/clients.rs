//! Status → Clients：ARP/ND 邻居 + DHCP 租约 + 静态绑定合并视图、网络唤醒。

use axum::{extract::State, response::Response, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::net::UdpSocket;

use crate::{cmd, state::AppState};

fn read_leases() -> HashMap<String, (String, String, i64)> {
    let mut m = HashMap::new();
    for p in [
        "/var/lib/misc/dnsmasq.leases",
        "/var/lib/dnsmasq/dnsmasq.leases",
        "/tmp/dhcp.leases",
    ] {
        if let Ok(s) = std::fs::read_to_string(p) {
            for line in s.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() >= 4 {
                    m.insert(
                        f[2].to_string(),
                        (
                            f[1].to_string(),
                            f[3].to_string(),
                            f[0].parse().unwrap_or(0),
                        ),
                    );
                }
            }
            break;
        }
    }
    m
}

pub async fn list(State(state): State<AppState>) -> Response {
    let leases = read_leases();
    let statics: HashMap<String, (String, String)> = {
        let db = state.db.lock().unwrap();
        let mut m = HashMap::new();
        if let Ok(mut stmt) = db.prepare("SELECT mac,ip,hostname FROM dhcp_static") {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            }) {
                for r in rows.flatten() {
                    m.insert(r.1.clone(), (r.0.clone(), r.2.clone()));
                }
            }
        }
        m
    };

    let neigh = cmd::run("ip", &["-j", "neigh"], 5).await;
    let mut items: Vec<Value> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();

    let add = |items: &mut Vec<Value>, seen: &mut HashMap<String, usize>, ip: String, mac: String, dev: String, st: String, hostname: String, lease: bool, is_static: bool, expires: i64| {
        if ip.is_empty() {
            return;
        }
        if let Some(&idx) = seen.get(&ip) {
            let it = &mut items[idx];
            if it["mac"] == "" && !mac.is_empty() {
                it["mac"] = json!(mac);
            }
            if it["hostname"] == "*" || it["hostname"] == "" {
                if !hostname.is_empty() && hostname != "*" {
                    it["hostname"] = json!(hostname);
                }
            }
            it["online"] = json!(true);
            return;
        }
        seen.insert(ip.clone(), items.len());
        items.push(json!({
            "ip": ip, "mac": mac, "dev": dev, "state": st,
            "hostname": if hostname.is_empty() || hostname == "*" { "".into() } else { hostname },
            "lease": lease, "static": is_static, "expires": expires, "online": true,
        }));
    };

    if let Ok(v) = serde_json::from_str::<Value>(&neigh.stdout) {
        if let Some(a) = v.as_array() {
            for n in a {
                let ip = n["dst"].as_str().unwrap_or("").to_string();
                let mac = n["lladdr"].as_str().unwrap_or("").to_string();
                let dev = n["dev"].as_str().unwrap_or("").to_string();
                let st = n["state"]
                    .as_array()
                    .map(|x| x.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("/"))
                    .unwrap_or_else(|| n["state"].as_str().unwrap_or("").to_string());
                let (lhn, _lmac, exp) = leases.get(&ip).cloned().unwrap_or_default();
                let (shn, _smac) = statics.get(&ip).cloned().unwrap_or_default();
                let hn = if !shn.is_empty() { shn } else { lhn };
                let is_static = statics.contains_key(&ip);
                add(&mut items, &mut seen, ip, mac, dev, st, hn, false, is_static, exp);
            }
        }
    }
    // 补充仅有租约/静态绑定的主机
    for (ip, (mac, hn, exp)) in &leases {
        add(&mut items, &mut seen, ip.clone(), mac.clone(), String::new(), "lease".into(), hn.clone(), true, false, *exp);
    }
    for (ip, (mac, hn)) in &statics {
        add(&mut items, &mut seen, ip.clone(), mac.clone(), String::new(), "static".into(), hn.clone(), false, true, 0);
    }
    items.sort_by(|a, b| {
        let pa: Vec<u8> = a["ip"].as_str().unwrap_or("").split('.').filter_map(|x| x.parse().ok()).collect();
        let pb: Vec<u8> = b["ip"].as_str().unwrap_or("").split('.').filter_map(|x| x.parse().ok()).collect();
        pa.cmp(&pb)
    });

    let online = items.iter().filter(|v| v["online"] == true && v["state"] != "lease" && v["state"] != "static").count();
    cmd::ok(json!({
        "items": items,
        "total": items.len(),
        "online": online,
        "leases": leases.len(),
        "statics": statics.len(),
    }))
}

/// 连接跟踪摘要（需 conntrack 工具）。
pub async fn connections() -> Response {
    if !cmd::which("conntrack") {
        return cmd::ok(json!({
            "available": false,
            "items": [],
            "msg": "未安装 conntrack：apt install conntrack"
        }));
    }
    let out = cmd::run("conntrack", &["-L"], 10).await;
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut proto_counts: HashMap<String, u32> = HashMap::new();
    for line in out.stdout.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() > 4 {
            *proto_counts.entry(f[2].to_string()).or_default() += 1;
        }
        if let Some(src) = line.split_whitespace().find(|x| x.starts_with("src=")) {
            *counts.entry(src.trim_start_matches("src=").to_string()).or_default() += 1;
        }
    }
    let mut items: Vec<Value> = counts
        .into_iter()
        .map(|(ip, count)| json!({ "ip": ip, "count": count }))
        .collect();
    items.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));
    items.truncate(50);
    cmd::ok(json!({
        "available": true,
        "items": items,
        "total": out.stdout.lines().count(),
        "protocols": proto_counts,
    }))
}

#[derive(Deserialize)]
pub struct WolReq {
    pub mac: String,
}

/// Wake-on-LAN：纯 Rust 构造 magic packet 并 UDP 广播，无需 etherwake。
pub async fn wol(Json(req): Json<WolReq>) -> Response {
    let mac = req.mac.trim().to_lowercase();
    if !cmd::is_mac(&mac) {
        return cmd::err("MAC 地址格式无效（应为 aa:bb:cc:dd:ee:ff）");
    }
    let bytes: Vec<u8> = mac
        .split(':')
        .map(|x| u8::from_str_radix(x, 16).unwrap_or(0))
        .collect();
    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&bytes);
    }
    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => return cmd::fail(format!("创建 UDP 套接字失败：{}", e)),
    };
    if sock.set_broadcast(true).is_err() {
        return cmd::fail("无法启用广播");
    }
    match sock.send_to(&packet, ("255.255.255.255", 9)).await {
        Ok(_) => cmd::ok(json!({ "ok": true, "msg": format!("已向 {} 发送唤醒包", mac) })),
        Err(e) => cmd::fail(format!("发送失败：{}", e)),
    }
}
