use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::{self, AppState};

#[derive(Serialize, Deserialize, Clone)]
pub struct Zone {
    pub name: String,
    pub ifaces: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Rule {
    pub id: i64,
    pub zone: String,
    pub proto: String,
    pub port: String,
    pub src: String,
    pub action: String,
    pub comment: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Forward {
    pub id: i64,
    pub proto: String,
    pub src_port: String,
    pub dest: String,
    pub dest_port: String,
    pub comment: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FwConfig {
    pub enabled: bool,
    pub masquerade: bool,
    pub allow_ping: bool,
    pub zones: Vec<Zone>,
    pub rules: Vec<Rule>,
    pub forwards: Vec<Forward>,
}

pub fn load_config(db: &Connection) -> FwConfig {
    let enabled = state::get_setting(db, "fw_enabled").unwrap_or_default() == "1";
    let masquerade = state::get_setting(db, "fw_masquerade").unwrap_or_default() != "0";
    let allow_ping = state::get_setting(db, "fw_allow_ping").unwrap_or_default() != "0";
    let mut zones = Vec::new();
    {
        let mut stmt = db.prepare("SELECT name, ifaces FROM fw_zones").unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok(Zone {
                    name: r.get(0)?,
                    ifaces: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or_default(),
                })
            })
            .unwrap();
        for z in rows.flatten() {
            zones.push(z);
        }
    }
    let mut rules = Vec::new();
    {
        let mut stmt = db
            .prepare("SELECT id,zone,proto,port,src,action,comment,enabled FROM fw_rules ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok(Rule {
                    id: r.get(0)?,
                    zone: r.get(1)?,
                    proto: r.get(2)?,
                    port: r.get(3)?,
                    src: r.get(4)?,
                    action: r.get(5)?,
                    comment: r.get(6)?,
                    enabled: r.get::<_, i64>(7)? == 1,
                })
            })
            .unwrap();
        for x in rows.flatten() {
            rules.push(x);
        }
    }
    let mut forwards = Vec::new();
    {
        let mut stmt = db
            .prepare("SELECT id,proto,src_port,dest,dest_port,comment,enabled FROM fw_forwards ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok(Forward {
                    id: r.get(0)?,
                    proto: r.get(1)?,
                    src_port: r.get(2)?,
                    dest: r.get(3)?,
                    dest_port: r.get(4)?,
                    comment: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? == 1,
                })
            })
            .unwrap();
        for x in rows.flatten() {
            forwards.push(x);
        }
    }
    FwConfig {
        enabled,
        masquerade,
        allow_ping,
        zones,
        rules,
        forwards,
    }
}

pub async fn get(State(state): State<AppState>) -> Response {
    let cfg = load_config(&state.db.lock().unwrap());
    Json(cfg).into_response()
}

pub async fn put(State(state): State<AppState>, Json(cfg): Json<FwConfig>) -> Response {
    let db = state.db.lock().unwrap();
    if let Err(e) = save_config(&db, &cfg) {
        return (axum::http::StatusCode::BAD_REQUEST, e).into_response();
    }
    Json(json!({"ok": true})).into_response()
}

fn save_config(db: &Connection, cfg: &FwConfig) -> Result<(), String> {
    for r in &cfg.rules {
        if !matches!(r.action.as_str(), "accept" | "drop") {
            return Err(format!("非法动作: {}", r.action));
        }
        if !matches!(r.proto.as_str(), "tcp" | "udp" | "icmp" | "all") {
            return Err(format!("非法协议: {}", r.proto));
        }
        if r.zone != "wan" && r.zone != "lan" {
            return Err(format!("未知 zone: {}", r.zone));
        }
    }
    for f in &cfg.forwards {
        if !matches!(f.proto.as_str(), "tcp" | "udp") {
            return Err(format!("端口转发协议须为 tcp/udp: {}", f.proto));
        }
        if f.dest.parse::<std::net::IpAddr>().is_err() {
            return Err(format!("目标地址无效: {}", f.dest));
        }
    }
    db.execute("DELETE FROM fw_zones", []).map_err(|e| e.to_string())?;
    db.execute("DELETE FROM fw_rules", []).map_err(|e| e.to_string())?;
    db.execute("DELETE FROM fw_forwards", []).map_err(|e| e.to_string())?;
    for z in &cfg.zones {
        db.execute(
            "INSERT INTO fw_zones(name,ifaces) VALUES(?1,?2)",
            rusqlite::params![z.name, serde_json::to_string(&z.ifaces).unwrap()],
        )
        .map_err(|e| e.to_string())?;
    }
    for r in &cfg.rules {
        db.execute(
            "INSERT INTO fw_rules(zone,proto,port,src,action,comment,enabled) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![r.zone, r.proto, r.port, r.src, r.action, r.comment, r.enabled as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    for f in &cfg.forwards {
        db.execute(
            "INSERT INTO fw_forwards(proto,src_port,dest,dest_port,comment,enabled) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![f.proto, f.src_port, f.dest, f.dest_port, f.comment, f.enabled as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    state::set_setting(db, "fw_enabled", if cfg.enabled { "1" } else { "0" });
    state::set_setting(db, "fw_masquerade", if cfg.masquerade { "1" } else { "0" });
    state::set_setting(db, "fw_allow_ping", if cfg.allow_ping { "1" } else { "0" });
    Ok(())
}

/// Generate nftables script. Safety rules:
/// - established/related always accepted
/// - when no wan interface is assigned, nothing is dropped
/// - when enabling with zero wan rules, a ssh(22) allow rule is auto-seeded
pub fn render_nft(cfg: &FwConfig) -> String {
    let wan_ifaces = cfg
        .zones
        .iter()
        .find(|z| z.name == "wan")
        .map(|z| z.ifaces.clone())
        .unwrap_or_default();
    let lan_ifaces = cfg
        .zones
        .iter()
        .find(|z| z.name == "lan")
        .map(|z| z.ifaces.clone())
        .unwrap_or_default();

    let set_elems = |v: &[String]| -> String {
        if v.is_empty() {
            "  # (empty)".into()
        } else {
            format!("  elements = {{ {} }}", v.join(", "))
        }
    };

    let mut s = String::from("#!/usr/sbin/nft -f\n# generated by bianwrt — do not edit\n\n");
    s.push_str("table inet bianwrt\ndelete table inet bianwrt\n\n");
    s.push_str("table inet bianwrt {\n");
    s.push_str("  set wan_ifs {\n    type ifname\n");
    s.push_str(&set_elems(&wan_ifaces));
    s.push_str("\n  }\n");
    s.push_str("  set lan_ifs {\n    type ifname\n");
    s.push_str(&set_elems(&lan_ifaces));
    s.push_str("\n  }\n\n");

    // input chain
    s.push_str("  chain input {\n    type filter hook input priority filter; policy accept;\n");
    s.push_str("    ct state established,related accept\n");
    s.push_str("    iifname @lan_ifs accept\n");
    if cfg.allow_ping {
        s.push_str("    meta l4proto icmp accept\n    meta l4proto ipv6-icmp accept\n");
    }
    if cfg.enabled && !wan_ifaces.is_empty() {
        for r in &cfg.rules {
            if !r.enabled || r.zone != "wan" {
                continue;
            }
            let proto = if r.proto == "all" { "meta l4proto { tcp, udp }".to_string() } else { format!("meta l4proto {}", r.proto) };
            let src = if r.src.is_empty() { String::new() } else { format!("ip saddr {} ", r.src) };
            let action = format!("iifname @wan_ifs {} {} dport {{ {} }} {} counter comment \"bianwrt:{}\"",
                                 proto, src, r.port, r.action, r.comment.replace('"', "'"));
            s.push_str(&format!("    {}\n", action.trim_start()));
        }
        s.push_str("    iifname @wan_ifs counter drop\n");
    }
    s.push_str("  }\n\n");

    // forward chain
    s.push_str("  chain forward {\n    type filter hook forward priority filter; policy accept;\n");
    s.push_str("    ct state established,related accept\n");
    s.push_str("    iifname @lan_ifs oifname @wan_ifs accept\n");
    s.push_str("    iifname @lan_ifs oifname @lan_ifs accept\n");
    if cfg.enabled && !wan_ifaces.is_empty() {
        for f in &cfg.forwards {
            if !f.enabled {
                continue;
            }
            let dport = if f.dest_port.is_empty() || f.dest_port == f.src_port {
                f.dest.clone()
            } else {
                format!("{}:{}", f.dest, f.dest_port)
            };
            s.push_str(&format!(
                "    iifname @wan_ifs oifname @lan_ifs {} dport {{ {} }} dnat ip to {} comment \"bianwrt:{}\"\n",
                f.proto, f.src_port, dport, f.comment.replace('"', "'")
            ));
        }
        s.push_str("    iifname @wan_ifs oifname @lan_ifs counter drop\n");
    }
    s.push_str("  }\n\n");

    // nat chains
    s.push_str("  chain dstnat {\n    type nat hook prerouting priority dstnat; policy accept;\n");
    if cfg.enabled && !wan_ifaces.is_empty() {
        for f in &cfg.forwards {
            if !f.enabled {
                continue;
            }
            let dport = if f.dest_port.is_empty() {
                f.dest.clone()
            } else {
                format!("{}:{}", f.dest, f.dest_port)
            };
            s.push_str(&format!(
                "    iifname @wan_ifs {} dport {{ {} }} dnat ip to {}\n",
                f.proto, f.src_port, dport
            ));
        }
    }
    s.push_str("  }\n\n");
    s.push_str("  chain srcnat {\n    type nat hook postrouting priority srcnat; policy accept;\n");
    if cfg.masquerade {
        s.push_str("    oifname @wan_ifs ip saddr != 127.0.0.0/8 masquerade\n");
    }
    s.push_str("  }\n}\n");
    s
}

#[axum::debug_handler]
pub async fn apply(State(state): State<AppState>) -> Response {
    let cfg = load_config(&state.db.lock().unwrap());
    if cfg.enabled && cfg.zones.iter().any(|z| z.name == "wan" && z.ifaces.is_empty()) {
        // empty wan set is fine; drop rules simply won't match — proceed
    }
    // safety: enabling with no wan accept rule -> seed ssh rule once
    if cfg.enabled
        && !cfg
            .rules
            .iter()
            .any(|r| r.zone == "wan" && r.action == "accept")
    {
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT INTO fw_rules(zone,proto,port,src,action,comment,enabled) VALUES('wan','tcp','22','','accept','自动加入: 保底 SSH 放行',1)",
                [],
            )
            .ok();
        }
        let cfg2 = {
            let db = state.db.lock().unwrap();
            load_config(&db)
        };
        return do_apply(&state, &cfg2).await;
    }
    do_apply(&state, &cfg).await
}

async fn do_apply(state: &AppState, cfg: &FwConfig) -> Response {
    let script = render_nft(cfg);
    let path = state.home.join("bianwrt.nft");
    if std::fs::write(&path, &script).is_err() {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "写入规则文件失败").into_response();
    }
    // syntax check first (dry run)
    let check = tokio::process::Command::new("nft")
        .args(["-c", "-f"])
        .arg(&path)
        .output()
        .await;
    let out = match check {
        Ok(o) => o,
        Err(_) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "nft 命令不可用，请安装 nftables (apt install nftables)",
            )
                .into_response()
        }
    };
    if !out.status.success() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("规则校验失败: {}", String::from_utf8_lossy(&out.stderr)),
        )
            .into_response();
    }
    let real = tokio::process::Command::new("nft")
        .args(["-f"])
        .arg(&path)
        .output()
        .await
        .unwrap();
    if !real.status.success() {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("应用失败: {}", String::from_utf8_lossy(&real.stderr)),
        )
            .into_response();
    }
    // persist for boot
    let _ = std::fs::copy(&path, "/etc/bianwrt/panel.nft");
    Json(json!({"ok": true, "script": script})).into_response()
}
