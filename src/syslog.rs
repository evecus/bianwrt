//! Status → System Log / Kernel Log：journalctl 读取 + WebSocket 实时跟随。

use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use std::process::Stdio;
use tokio::process::Command;

use crate::cmd;

#[derive(Debug, Deserialize, Default)]
pub struct LogQuery {
    pub lines: Option<u32>,
    pub unit: Option<String>,
    pub priority: Option<String>,
    pub grep: Option<String>,
    pub kernel: Option<bool>,
}

fn line_count(q: &LogQuery) -> usize {
    q.lines.unwrap_or(200).clamp(10, 2000) as usize
}

fn journal_args(q: &LogQuery, follow: bool) -> Vec<String> {
    let mut a = vec!["--no-pager".to_string(), "-o".into(), "short-iso".into()];
    if q.kernel.unwrap_or(false) {
        a.push("-k".into());
    }
    if follow {
        a.push("-f".into());
        a.push("-n".into());
        a.push("20".into());
    } else {
        a.push("-n".into());
        a.push(line_count(q).to_string());
    }
    if let Some(u) = &q.unit {
        if cmd::safe(u, 128) {
            a.push("-u".into());
            a.push(u.clone());
        }
    }
    if let Some(p) = &q.priority {
        let ok = matches!(
            p.as_str(),
            "emerg" | "alert" | "crit" | "err" | "warning" | "notice" | "info" | "debug"
                | "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7"
        );
        if ok {
            a.push("-p".into());
            a.push(p.clone());
        }
    }
    a
}

fn grep_filter(lines: Vec<String>, q: &LogQuery) -> Vec<String> {
    match &q.grep {
        Some(g) if !g.trim().is_empty() && g.len() <= 128 => {
            let g = g.to_lowercase();
            lines
                .into_iter()
                .filter(|l| l.to_lowercase().contains(&g))
                .collect()
        }
        _ => lines,
    }
}

async fn fallback(q: &LogQuery) -> (String, Vec<String>) {
    let n = line_count(q);
    if q.kernel.unwrap_or(false) {
        let mut o = cmd::run("dmesg", &["--color=never", "--time-format", "iso"], 8).await;
        if !o.ok {
            o = cmd::run("dmesg", &["--color=never"], 8).await;
        }
        if o.ok {
            return ("dmesg".into(), grep_filter(cmd::tail_lines(&o.stdout, n), q));
        }
        return (
            "unavailable".into(),
            vec!["内核日志不可用（需要 journald 或 dmesg 权限）".into()],
        );
    }
    for p in ["/var/log/syslog", "/var/log/messages", "/var/log/system.log"] {
        if let Ok(s) = std::fs::read_to_string(p) {
            return (p.into(), grep_filter(cmd::tail_lines(&s, n), q));
        }
    }
    (
        "unavailable".into(),
        vec![
            "未找到日志：journalctl 不可用且 /var/log/syslog 不存在。".into(),
            "提示：Debian/Ubuntu 可安装 systemd-journald 或 rsyslog。".into(),
        ],
    )
}

pub async fn read(Query(q): Query<LogQuery>) -> Response {
    let args = journal_args(&q, false);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = cmd::run("journalctl", &refs, 10).await;
    if out.ok && !out.stdout.trim().is_empty() {
        let lines: Vec<String> = out.stdout.lines().map(|s| s.to_string()).collect();
        return cmd::ok(json!({
            "source": "journald",
            "lines": grep_filter(lines, &q),
            "available": true
        }));
    }
    let (source, lines) = fallback(&q).await;
    cmd::ok(json!({ "source": source, "lines": lines, "available": source != "unavailable" }))
}

/// 实时日志流：浏览器断开时 kill 掉 `journalctl -f`。
pub async fn follow(ws: WebSocketUpgrade, Query(q): Query<LogQuery>) -> Response {
    ws.on_upgrade(move |socket| async move {
        let (mut sink, mut stream) = socket.split();
        let args = journal_args(&q, true);
        let mut child = match Command::new("journalctl")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let m = format!("[bianwrt] 无法启动 journalctl: {}", e);
                let _ = sink.send(Message::Text(m.into())).await;
                return;
            }
        };
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut buf = String::new();
        let _ = sink
            .send(Message::Text(
                "[bianwrt] 实时日志已连接（关闭页面即停止跟随）".into(),
            ))
            .await;
        loop {
            tokio::select! {
                r = reader.read_line(&mut buf) => {
                    match r {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = buf.trim_end().to_string();
                            buf.clear();
                            if let Some(g) = &q.grep {
                                if !g.is_empty() && !line.to_lowercase().contains(&g.to_lowercase()) {
                                    continue;
                                }
                            }
                            if sink.send(Message::Text(line.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                msg = stream.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                        _ => {}
                    }
                }
            }
        }
        let _ = child.kill().await;
    })
}
