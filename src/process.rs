//! Status → Processes：进程列表、排序、发信号。

use axum::{response::Response, Json};
use serde::Deserialize;
use serde_json::json;

use crate::cmd;

pub async fn list() -> Response {
    let out = cmd::run(
        "ps",
        &[
            "-eo",
            "pid=,ppid=,user=,stat=,pcpu=,pmem=,etime=,comm=,args=",
            "--sort=-pcpu",
        ],
        8,
    )
    .await;
    if !out.ok {
        return cmd::fail(format!("ps 执行失败: {}", out.stderr));
    }
    let mut items = Vec::new();
    for line in out.stdout.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid), Some(user), Some(stat), Some(cpu), Some(mem), Some(etime), Some(comm)) = (
            it.next(), it.next(), it.next(), it.next(),
            it.next(), it.next(), it.next(), it.next(),
        ) else { continue };
        let args = it.collect::<Vec<_>>().join(" ");
        items.push(json!({
            "pid": pid.parse::<i64>().unwrap_or(0),
            "ppid": ppid.parse::<i64>().unwrap_or(0),
            "user": user,
            "stat": stat,
            "cpu": cpu.parse::<f64>().unwrap_or(0.0),
            "mem": mem.parse::<f64>().unwrap_or(0.0),
            "etime": etime,
            "comm": comm,
            "args": args,
        }));
    }
    cmd::ok(json!({ "items": items, "total": items.len() }))
}

#[derive(Deserialize)]
pub struct KillReq {
    pub pid: i64,
    pub signal: Option<String>,
}

pub async fn kill(Json(req): Json<KillReq>) -> Response {
    if req.pid <= 1 {
        return cmd::err("拒绝向 PID 1 发送信号");
    }
    let sig = req.signal.unwrap_or_else(|| "TERM".into());
    if !matches!(
        sig.as_str(),
        "TERM" | "KILL" | "HUP" | "INT" | "USR1" | "USR2" | "STOP" | "CONT"
    ) {
        return cmd::err("不支持的信号");
    }
    let flag = format!("-{}", sig);
    let pid = req.pid.to_string();
    let out = cmd::run("kill", &[&flag, &pid], 5).await;
    if !out.ok {
        return cmd::fail(format!("{}", out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true, "msg": format!("已向 {} 发送 {} 信号", pid, sig) }))
}
