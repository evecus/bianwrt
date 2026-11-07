//! System → Scheduled Tasks：crontab 托管块（不破坏用户原有任务）。

use axum::{extract::State, response::Response, Json};
use serde::Deserialize;
use serde_json::json;

use crate::{cmd, state::AppState};

const BEGIN: &str = "# BEGIN bianwrt-managed";
const END: &str = "# END bianwrt-managed";

fn split_managed(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut managed = Vec::new();
    let mut other = Vec::new();
    let mut inside = false;
    for line in raw.lines() {
        if line.trim() == BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == END {
            inside = false;
            continue;
        }
        if inside {
            managed.push(line.to_string());
        } else {
            other.push(line.to_string());
        }
    }
    (managed, other)
}

pub async fn get() -> Response {
    let out = cmd::run("crontab", &["-l"], 5).await;
    let raw = if out.ok { out.stdout } else { String::new() };
    let (managed, other) = split_managed(&raw);
    cmd::ok(json!({
        "managed": managed,
        "other": other,
        "available": cmd::which("crontab"),
        "raw": raw,
    }))
}

#[derive(Deserialize)]
pub struct SaveReq {
    pub lines: Vec<String>,
}

pub async fn put(
    State(state): State<AppState>,
    Json(req): Json<SaveReq>,
) -> Response {
    if req.lines.len() > 200 {
        return cmd::err("计划任务数量超过上限（200）");
    }
    for l in &req.lines {
        if l.len() > 512
            || l.chars().any(|c| c == '\n' || c == '\r' || c == '\0')
            || l.contains(BEGIN)
            || l.contains(END)
        {
            return cmd::err("计划任务包含非法字符或标记");
        }
    }
    let out = cmd::run("crontab", &["-l"], 5).await;
    let raw = if out.ok { out.stdout } else { String::new() };
    let (_, other) = split_managed(&raw);

    let mut content = String::new();
    for l in other {
        let l = l.trim_end();
        if !l.is_empty() {
            content.push_str(l);
            content.push('\n');
        }
    }
    if !req.lines.is_empty() {
        content.push_str(BEGIN);
        content.push('\n');
        for l in &req.lines {
            content.push_str(l.trim());
            content.push('\n');
        }
        content.push_str(END);
        content.push('\n');
    }

    let path = state.data_dir.join("crontab.bianwrt");
    if std::fs::write(&path, &content).is_err() {
        return cmd::fail("写入临时 crontab 失败");
    }
    let p = path.to_string_lossy().to_string();
    let r = cmd::run("crontab", &[&p], 8).await;
    if !r.ok {
        return cmd::fail(format!("安装 crontab 失败: {}", r.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true, "applied": content }))
}
