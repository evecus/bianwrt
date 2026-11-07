//! 通用命令执行与参数校验。
//! 所有面板对系统的操作都必须经过这里：不经过 shell、带超时、带白名单参数校验。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    #[allow(dead_code)]
    pub code: i32,
}

/// 执行外部命令，绝不通过 shell；超时后 kill_on_drop 保证不残留子进程。
pub async fn run(program: &str, args: &[&str], secs: u64) -> Out {
    let mut c = Command::new(program);
    c.args(args).kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(secs), c.output()).await {
        Ok(Ok(o)) => Out {
            ok: o.status.success(),
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Ok(Err(e)) => Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("无法执行 {}: {}", program, e),
        },
        Err(_) => Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("命令超时（{}s）", secs),
        },
    }
}

/// 执行需要 stdin 输入的命令（如 `wg pubkey`）。
pub async fn run_stdin(program: &str, args: &[&str], input: &[u8], secs: u64) -> Out {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut c = Command::new(program);
    c.args(args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match c.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Out {
                ok: false,
                code: -1,
                stdout: String::new(),
                stderr: e.to_string(),
            }
        }
    };
    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(input).await;
    }
    match tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output()).await {
        Ok(Ok(o)) => Out {
            ok: o.status.success(),
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Ok(Err(e)) => Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: e.to_string(),
        },
        Err(_) => Out {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: format!("命令超时（{}s）", secs),
        },
    }
}

pub fn ok(v: Value) -> Response {
    Json(v).into_response()
}

pub fn err(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, msg.into()).into_response()
}

pub fn fail(msg: impl Into<String>) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, msg.into()).into_response()
}

/// 单值参数白名单：接口名 / unit 名 / 时区 / IP / 端口等。
/// 明确排除 shell 元字符、空白、引号、反斜杠、控制字符。
pub fn safe(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-.:/@+=".contains(c))
}

/// 多级相对路径（包内条目 / 备份条目）：禁止绝对路径、`..`、反斜杠与控制字符。
pub fn safe_rel(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.starts_with('/')
        && !s.contains('\\')
        && !s.chars().any(|c| c.is_control())
        && s.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// manifest 文本字段：禁止控制字符、换行、引号（防止注入 systemd unit）。
pub fn safe_manifest_text(s: &str, max: usize) -> bool {
    s.len() <= max
        && !s.chars().any(|c| c.is_control() || c == '"' || c == '\'' || c == '\0')
}

/// 主机名 / 域名 / IP（可含端口分隔符），供 ping、nslookup、DNS 记录使用。
pub fn is_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('-')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c))
}

/// IPv4/IPv6 地址或 CIDR。
pub fn is_ip_or_cidr(s: &str) -> bool {
    let (addr, prefix) = match s.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (s, None),
    };
    if addr.parse::<std::net::IpAddr>().is_err() {
        return false;
    }
    match prefix {
        None => true,
        Some(p) => p.parse::<u8>().map(|n| n <= 128).unwrap_or(false),
    }
}

pub fn is_ip(s: &str) -> bool {
    s.parse::<std::net::IpAddr>().is_ok()
}

/// 自由文本（SSID / 密码 / 备注）：允许 UTF-8，但禁止控制字符与换行。
pub fn safe_text(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.chars().any(|c| c.is_control() || c == '\0')
}

/// HTTP(S) URL 模板：禁止空白与控制字符（参数会以单个 argv 传给 curl，不经 shell）。
pub fn safe_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && s.len() <= 1024
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub fn is_mac(s: &str) -> bool {
    let p: Vec<&str> = s.split(':').collect();
    p.len() == 6 && p.iter().all(|x| x.len() == 2 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn is_domain(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('.')
        && !s.contains('/')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-*".contains(c))
}

pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn tail_lines(s: &str, n: usize) -> Vec<String> {
    let all: Vec<&str> = s.lines().collect();
    let start = all.len().saturating_sub(n);
    all[start..].iter().map(|x| x.to_string()).collect()
}

/// 检查命令是否存在（纯 Rust 遍历 PATH，不调用 shell）。
pub fn which(name: &str) -> bool {
    if name.is_empty() || name.contains('/') {
        return false;
    }
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .any(|d| d.join(name).is_file())
        })
        .unwrap_or(false)
}
