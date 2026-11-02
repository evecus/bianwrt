use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

/// Read-only view of interfaces & routes via `ip -j`.
/// Network modification is deliberately out of scope in v1 (avoid bricking servers).

async fn run_ip(args: &[&str]) -> Value {
    let out = tokio::process::Command::new("ip")
        .args(args)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            serde_json::from_slice(&o.stdout).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

pub async fn interfaces() -> Response {
    let addrs = run_ip(&["-j", "-d", "addr"]).await;
    if addrs.is_null() {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "ip 命令不可用").into_response();
    }
    // enrich with link info
    let links = run_ip(&["-j", "-d", "link"]).await;
    Json(serde_json::json!({ "addrs": addrs, "links": links })).into_response()
}

pub async fn routes() -> Response {
    let v4 = run_ip(&["-j", "route"]).await;
    let v6 = run_ip(&["-j", "-6", "route"]).await;
    Json(serde_json::json!({ "v4": v4, "v6": v6 })).into_response()
}
