//! System → Startup：systemd 服务列表、启停、开机自启、状态详情。

use axum::{extract::Query, response::Response, Json};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::cmd;

async fn systemctl(args: &[&str], secs: u64) -> cmd::Out {
    let mut plain = vec!["--no-pager", "--no-legend", "--plain"];
    plain.extend_from_slice(args);
    let out = cmd::run("systemctl", &plain, secs).await;
    if out.ok {
        return out;
    }
    // 老版本 systemd 不支持 --plain
    let mut plain = vec!["--no-pager", "--no-legend"];
    plain.extend_from_slice(args);
    cmd::run("systemctl", &plain, secs).await
}

pub async fn list() -> Response {
    let files = systemctl(&["list-unit-files", "--type=service"], 12).await;
    let units = systemctl(&["list-units", "--type=service", "--all"], 12).await;
    if !files.ok && !units.ok {
        return cmd::fail("systemctl 不可用（需要 systemd 系统）");
    }

    let mut map: HashMap<String, Map<String, Value>> = HashMap::new();
    for line in files.stdout.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() {
            continue;
        }
        let unit = f[0].to_string();
        let e = map.entry(unit.clone()).or_default();
        e.insert("unit".into(), json!(unit));
        e.insert("enabled".into(), json!(f.get(1).unwrap_or(&"unknown")));
        e.insert("active".into(), json!("inactive"));
        e.insert("sub".into(), json!("dead"));
        e.insert("description".into(), json!(""));
    }
    for line in units.stdout.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() {
            continue;
        }
        let unit = f[0].to_string();
        let e = map.entry(unit.clone()).or_default();
        e.insert("unit".into(), json!(unit));
        e.insert("active".into(), json!(f.get(2).unwrap_or(&"unknown")));
        e.insert("sub".into(), json!(f.get(3).unwrap_or(&"")));
        e.insert(
            "description".into(),
            json!(if f.len() > 4 { f[4..].join(" ") } else { String::new() }),
        );
        e.entry(String::from("enabled"))
            .or_insert_with(|| json!("unknown"));
    }
    let mut items: Vec<Value> = map.into_values().map(Value::Object).collect();
    items.sort_by(|a, b| a["unit"].as_str().cmp(&b["unit"].as_str()));
    cmd::ok(json!({ "items": items, "total": items.len() }))
}

#[derive(Deserialize)]
pub struct ActionReq {
    pub unit: String,
    pub op: String,
}

pub async fn action(Json(req): Json<ActionReq>) -> Response {
    if !cmd::safe(&req.unit, 128) {
        return cmd::err("服务名包含非法字符");
    }
    if !req.unit.ends_with(".service") && !req.unit.contains('@') {
        return cmd::err("仅支持 .service 单元");
    }
    let allowed = [
        "start", "stop", "restart", "reload", "enable", "disable", "mask", "unmask",
        "try-restart", "reload-or-restart",
    ];
    if !allowed.contains(&req.op.as_str()) {
        return cmd::err("不支持的操作");
    }
    let out = cmd::run("systemctl", &[&req.op, &req.unit], 40).await;
    if !out.ok {
        return cmd::fail(format!(
            "{} {} 失败: {}",
            req.op,
            req.unit,
            out.stderr.trim()
        ));
    }
    cmd::ok(json!({ "ok": true, "msg": format!("{} {} 成功", req.unit, req.op) }))
}

#[derive(Deserialize)]
pub struct StatusQuery {
    pub unit: String,
}

pub async fn status(Query(q): Query<StatusQuery>) -> Response {
    if !cmd::safe(&q.unit, 128) {
        return cmd::err("服务名包含非法字符");
    }
    let out = cmd::run("systemctl", &["status", "--no-pager", "-l", "-n", "30", &q.unit], 10).await;
    cmd::ok(json!({
        "text": if out.stdout.is_empty() { out.stderr } else { out.stdout },
        "ok": out.ok
    }))
}

/// 服务日志：journalctl -u <unit>（journald 不可用时回退 systemctl status）。
#[derive(Deserialize)]
pub struct LogQuery {
    pub unit: String,
    #[serde(default)]
    pub lines: Option<u32>,
}

pub async fn log(Query(q): Query<LogQuery>) -> Response {
    if !valid_unit(&q.unit) {
        return cmd::err("服务名包含非法字符");
    }
    let n = q.lines.unwrap_or(200).clamp(10, 2000).to_string();
    let text: String;
    let source: &str;
    if cmd::which("journalctl") {
        let out = cmd::run(
            "journalctl",
            &[
                "-u",
                &q.unit,
                "-n",
                &n,
                "--no-pager",
                "--no-hostname",
                "-o",
                "short-iso",
            ],
            15,
        )
        .await;
        text = if out.stdout.trim().is_empty() {
            format!("(journalctl 无输出)\n{}", out.stderr.trim())
        } else {
            out.stdout
        };
        source = "journalctl";
    } else {
        let out = cmd::run("systemctl", &["status", "--no-pager", "-l", "-n", &n, &q.unit], 12).await;
        text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        source = "systemctl status（journalctl 不可用）";
    }
    cmd::ok(json!({ "text": text, "source": source, "lines": n }))
}

/// 开机自启开关：只改变 enable/disable，不启停当前服务。
#[derive(Deserialize)]
pub struct AutostartReq {
    pub unit: String,
    pub enabled: bool,
}

pub async fn autostart(Json(req): Json<AutostartReq>) -> Response {
    if !valid_unit(&req.unit) {
        return cmd::err("服务名包含非法字符");
    }
    let op = if req.enabled { "enable" } else { "disable" };
    let out = cmd::run("systemctl", &[op, &req.unit], 40).await;
    if !out.ok {
        return cmd::fail(format!(
            "{} {} 失败: {}",
            op,
            req.unit,
            if out.stderr.trim().is_empty() {
                out.stdout.trim()
            } else {
                out.stderr.trim()
            }
        ));
    }
    let state = cmd::run("systemctl", &["is-enabled", &req.unit], 10).await;
    let enabled = state.stdout.trim().trim_end_matches('\n').to_string();
    cmd::ok(json!({
        "ok": true,
        "enabled": enabled,
        "msg": format!("{} 已{}开机自启", req.unit, if req.enabled { "启用" } else { "禁用" })
    }))
}

fn valid_unit(s: &str) -> bool {
    cmd::safe(s, 128) && (s.ends_with(".service") || s.contains('@'))
}
