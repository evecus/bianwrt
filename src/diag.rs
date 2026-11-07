//! Network → Diagnostics：ping / traceroute / nslookup / TCP 端口 / HTTP 探测。

use axum::{response::Response, Json};
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

use crate::cmd;

fn result(ok: bool, tool: &str, output: String) -> Response {
    cmd::ok(json!({ "ok": ok, "tool": tool, "output": output }))
}

fn net_host(h: &str, port: u16) -> String {
    if h.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{}]:{}", h, port)
    } else {
        format!("{}:{}", h, port)
    }
}

fn is_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && s.len() <= 512
        && !s.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || "`$;|&<>'\"\\(){}".contains(c)
        })
}

pub async fn tools() -> Response {
    cmd::ok(json!({
        "ping": cmd::which("ping"),
        "ping6": cmd::which("ping6"),
        "traceroute": cmd::which("traceroute"),
        "tracepath": cmd::which("tracepath"),
        "nslookup": cmd::which("nslookup"),
        "dig": cmd::which("dig"),
        "curl": cmd::which("curl"),
        "mtr": cmd::which("mtr"),
    }))
}

#[derive(Deserialize)]
pub struct PingReq {
    pub host: String,
    pub count: Option<u32>,
    pub family: Option<String>,
}

pub async fn ping(Json(req): Json<PingReq>) -> Response {
    if !cmd::is_host(&req.host) {
        return cmd::err("目标地址非法");
    }
    let count = req.count.unwrap_or(4).clamp(1, 20).to_string();
    let mut args = vec!["-c", &count, "-W", "2", "-n"];
    let fam = req.family.unwrap_or_else(|| "4".into());
    match fam.as_str() {
        "4" => args.push("-4"),
        "6" => args.push("-6"),
        _ => {}
    }
    args.push(&req.host);
    let out = cmd::run("ping", &args, 30).await;
    let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
    result(out.ok, "ping", text)
}

#[derive(Deserialize)]
pub struct TraceReq {
    pub host: String,
    pub hops: Option<u32>,
}

pub async fn traceroute(Json(req): Json<TraceReq>) -> Response {
    if !cmd::is_host(&req.host) {
        return cmd::err("目标地址非法");
    }
    let hops = req.hops.unwrap_or(15).clamp(1, 30).to_string();
    if cmd::which("traceroute") {
        let out = cmd::run(
            "traceroute",
            &["-m", &hops, "-w", "2", "-q", "1", "-n", &req.host],
            60,
        )
        .await;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        return result(out.ok, "traceroute", text);
    }
    if cmd::which("tracepath") {
        let out = cmd::run("tracepath", &["-m", &hops, "-n", &req.host], 60).await;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        return result(out.ok, "tracepath", text);
    }
    result(
        false,
        "traceroute",
        "未找到 traceroute / tracepath，请先安装：apt install traceroute".into(),
    )
}

#[derive(Deserialize)]
pub struct LookupReq {
    pub host: String,
    pub server: Option<String>,
}

pub async fn nslookup(Json(req): Json<LookupReq>) -> Response {
    if !cmd::is_host(&req.host) {
        return cmd::err("域名非法");
    }
    if let Some(s) = &req.server {
        if !cmd::is_ip(s) {
            return cmd::err("DNS 服务器地址非法");
        }
    }
    if cmd::which("nslookup") {
        let mut args = vec![req.host.as_str()];
        if let Some(s) = &req.server {
            args.push(s.as_str());
        }
        let out = cmd::run("nslookup", &args, 15).await;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        return result(out.ok, "nslookup", text);
    }
    if cmd::which("dig") {
        let target = match &req.server {
            Some(s) => format!("@{}", s),
            None => req.host.clone(),
        };
        let args: Vec<&str> = if req.server.is_some() {
            vec!["+noall", "+answer", &target, &req.host]
        } else {
            vec!["+noall", "+answer", &target]
        };
        let out = cmd::run("dig", &args, 15).await;
        let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
        return result(out.ok, "dig", text);
    }
    // 纯 Rust 兜底：使用系统解析器
    match tokio::net::lookup_host(net_host(&req.host, 0)).await {
        Ok(addrs) => {
            let list: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
            if list.is_empty() {
                return result(false, "getaddrinfo", "未解析到任何地址".into());
            }
            result(true, "getaddrinfo", format!("{}\n\n（未安装 nslookup/dig，使用系统解析器）", list.join("\n")))
        }
        Err(e) => result(false, "getaddrinfo", e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct TcpReq {
    pub host: String,
    pub port: u16,
    pub timeout: Option<u64>,
}

pub async fn tcp(Json(req): Json<TcpReq>) -> Response {
    if !cmd::is_host(&req.host) || req.port == 0 {
        return cmd::err("目标地址或端口非法");
    }
    let secs = req.timeout.unwrap_or(3).clamp(1, 15);
    let target = net_host(&req.host, req.port);
    let t0 = Instant::now();
    let r = tokio::time::timeout(Duration::from_secs(secs), TcpStream::connect(&target)).await;
    let ms = t0.elapsed().as_millis();
    match r {
        Ok(Ok(_)) => result(
            true,
            "tcp",
            format!("{} 可连接，耗时 {} ms", target, ms),
        ),
        Ok(Err(e)) => result(false, "tcp", format!("{} 连接失败：{}（{} ms）", target, e, ms)),
        Err(_) => result(false, "tcp", format!("{} 连接超时（{} s）", target, secs)),
    }
}

#[derive(Deserialize)]
pub struct HttpReq {
    pub url: String,
    pub timeout: Option<u64>,
}

pub async fn http(Json(req): Json<HttpReq>) -> Response {
    if !is_url(&req.url) {
        return cmd::err("URL 非法（仅支持 http/https 且不含 shell 元字符）");
    }
    if !cmd::which("curl") {
        return result(false, "curl", "未找到 curl：apt install curl".into());
    }
    let secs = req.timeout.unwrap_or(8).clamp(1, 30).to_string();
    let fmt = "HTTP %{http_code} | 总耗时 %{time_total}s | DNS %{time_namelookup}s | 连接 %{time_connect}s | 大小 %{size_download}B";
    let out = cmd::run(
        "curl",
        &["-sS", "-L", "-o", "/dev/null", "-w", fmt, "-m", &secs, &req.url],
        secs.parse::<u64>().unwrap_or(8) + 3,
    )
    .await;
    let text = format!("{}\n{}", out.stdout.trim(), out.stderr.trim()).trim().to_string();
    result(out.ok, "curl", text)
}
