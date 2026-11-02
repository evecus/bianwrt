use axum::extract::ws::{Message, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{SinkExt, StreamExt};
use serde_json::json;

use crate::state::AppState;

pub async fn installed(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    let out = tokio::process::Command::new("dpkg-query")
        .args([
            "-W",
            "-f=${Package}\t${Version}\t${db:Status-Abbrev}\t${binary:Summary}\n",
        ])
        .output()
        .await;
    let Ok(out) = out else {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "dpkg-query 不可用").into_response();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let qlc = q.get("q").map(|s| s.to_lowercase()).unwrap_or_default();
    let items: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| l.contains('\t'))
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            if f.len() < 4 {
                return None;
            }
            let name = f[0].to_string();
            if !qlc.is_empty() && !name.contains(&qlc) && !f[3].to_lowercase().contains(&qlc) {
                return None;
            }
            let installed = f[2].trim().starts_with("ii");
            Some(json!({
                "name": name, "version": f[1], "installed": installed,
                "summary": f[3].trim(),
            }))
        })
        .take(500)
        .collect();
    Json(json!({"items": items, "count": items.len()})).into_response()
}

pub async fn search(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Response {
    let term = q.get("q").cloned().unwrap_or_default();
    if term.is_empty() {
        return Json(json!({"items": []})).into_response();
    }
    let out = tokio::process::Command::new("apt-cache")
        .args(["search", &term])
        .output()
        .await;
    let Ok(out) = out else {
        return Json(json!({"items": []})).into_response();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let items: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| !l.starts_with(' '))
        .take(60)
        .filter_map(|l| {
            let (name, desc) = l.split_once(" - ")?;
            Some(json!({"name": name, "summary": desc}))
        })
        .collect();
    Json(json!({"items": items})).into_response()
}

#[derive(serde::Deserialize)]
pub struct ActionReq {
    pub op: String, // install | remove | purge
    pub packages: Vec<String>,
}

pub async fn action(State(state): State<AppState>, Json(req): Json<ActionReq>) -> Response {
    if !matches!(req.op.as_str(), "install" | "remove" | "purge") {
        return (axum::http::StatusCode::BAD_REQUEST, "不支持的操作").into_response();
    }
    if req.packages.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "未指定软件包").into_response();
    }
    for p in &req.packages {
        if p.contains(|c: char| !(c.is_ascii_alphanumeric() || "+-._".contains(c))) {
            return (axum::http::StatusCode::BAD_REQUEST, format!("非法包名: {}", p)).into_response();
        }
    }
    let mut running = state.pkg_running.lock().await;
    if *running {
        return (axum::http::StatusCode::CONFLICT, "已有任务在执行中").into_response();
    }
    *running = true;
    drop(running);

    let st = state.clone();
    tokio::spawn(async move {
        let _ = st.pkg_tx.send(format!("[bianwrt] 开始: apt-get -y {} {}", req.op, req.packages.join(" ")));
        let mut cmd = tokio::process::Command::new("apt-get");
        cmd.args(["-y", &req.op]).args(&req.packages);
        let _ = cmd.stdin(std::process::Stdio::null());
        let _ = cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let env = std::env::var("DEBIAN_FRONTEND").unwrap_or_default();
        cmd.env("DEBIAN_FRONTEND", if env.is_empty() { "noninteractive" } else { &env });
        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();
                let tx1 = st.pkg_tx.clone();
                let tx2 = st.pkg_tx.clone();
                let r1 = tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    let mut reader = tokio::io::BufReader::new(stdout);
                    let mut buf = String::new();
                    loop {
                        match reader.read_line(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                let line = buf.trim_end().to_string();
                                let _ = tx1.send(line);
                                buf.clear();
                            }
                        }
                    }
                });
                let r2 = tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    let mut reader = tokio::io::BufReader::new(stderr);
                    let mut buf = String::new();
                    loop {
                        match reader.read_line(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                let line = buf.trim_end().to_string();
                                let _ = tx2.send(line);
                                buf.clear();
                            }
                        }
                    }
                });
                let _ = child.wait().await;
                let _ = r1.await;
                let _ = r2.await;
            }
            Err(e) => {
                let _ = st.pkg_tx.send(format!("[bianwrt] 启动失败: {}", e));
            }
        }
        let _ = st.pkg_tx.send(format!("[bianwrt] 任务结束"));
        *st.pkg_running.lock().await = false;
    });
    Json(json!({"ok": true, "msg": "任务已启动，请通过 WebSocket 接收输出"})).into_response()
}

pub async fn ws(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| async move {
        let mut rx = state.pkg_tx.subscribe();
        let (mut tx_out, mut rx_in) = socket.split();
        let send_task = tokio::spawn(async move {
            while let Ok(line) = rx.recv().await {
                if tx_out.send(Message::Text(line.into())).await.is_err() {
                    break;
                }
            }
        });
        // drain client messages; close on disconnect
        while let Some(Ok(msg)) = rx_in.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
        send_task.abort();
    })
}
