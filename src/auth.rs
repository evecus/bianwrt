use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

use crate::state::AppState;

const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Serialize, Deserialize, Clone)]
pub struct Admin {
    pub username: String,
    pub salt: String,
    pub hash: String,
}

#[derive(Clone)]
pub struct Session {
    #[allow(dead_code)]
    pub username: String,
    pub expires: Instant,
}

pub fn hash_password(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    let mut out = hasher.finalize_reset().to_vec();
    for _ in 0..9999 {
        hasher.update(&out);
        hasher.update(salt.as_bytes());
        out = hasher.finalize_reset().to_vec();
    }
    hex::encode(out)
}

pub fn gen_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn get_admin(state: &AppState) -> Option<Admin> {
    let db = state.db.lock().unwrap();
    let mut stmt = db
        .prepare("SELECT value FROM settings WHERE key='admin'")
        .ok()?;
    stmt.query_row([], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

#[derive(Deserialize)]
pub struct SetupReq {
    pub username: String,
    pub password: String,
}

pub async fn setup(State(state): State<AppState>, Json(req): Json<SetupReq>) -> Response {
    if get_admin(&state).is_some() {
        return (StatusCode::BAD_REQUEST, "already initialized").into_response();
    }
    if req.password.len() < 6 {
        return (StatusCode::BAD_REQUEST, "密码至少 6 位").into_response();
    }
    let salt = gen_token();
    let admin = Admin {
        username: req.username,
        salt: salt.clone(),
        hash: hash_password(&req.password, &salt),
    };
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES('admin',?1)",
        [serde_json::to_string(&admin).unwrap()],
    )
    .ok();
    (StatusCode::OK, "ok").into_response()
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

pub async fn login(State(state): State<AppState>, Json(req): Json<LoginReq>) -> Response {
    let Some(admin) = get_admin(&state) else {
        return (StatusCode::BAD_REQUEST, "请先完成初始化").into_response();
    };
    if admin.username != req.username || admin.hash != hash_password(&req.password, &admin.salt) {
        return (StatusCode::UNAUTHORIZED, "用户名或密码错误").into_response();
    }
    let token = gen_token();
    state.sessions.lock().await.insert(
        token.clone(),
        Session {
            username: req.username,
            expires: Instant::now() + SESSION_TTL,
        },
    );
    Json(serde_json::json!({ "token": token })).into_response()
}

pub async fn status(State(state): State<AppState>) -> Response {
    let admin = get_admin(&state);
    Json(serde_json::json!({
        "need_setup": admin.is_none(),
        "username": admin.map(|a| a.username),
    }))
    .into_response()
}

pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Some(tok) = bearer(&headers) {
        state.sessions.lock().await.remove(&tok);
    }
    (StatusCode::OK, "ok").into_response()
}

fn bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

/// extract token from Authorization header or ?token= query (for WebSocket)
fn token_of(req: &Request) -> Option<String> {
    if let Some(t) = bearer(req.headers()) {
        return Some(t);
    }
    req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("token="))
            .map(|t| t.to_string())
    })
}

/// Auth middleware: protects /api routes except /api/auth/*
pub async fn auth_mw(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    // nested router strips the /api prefix, so accept both forms
    if path.starts_with("/api/auth/") || path.starts_with("/auth/") {
        return next.run(req).await;
    }
    let ok = token_of(&req)
        .and_then(|t| state.sessions.try_lock().ok().map(|m| m.contains_key(&t)))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "未登录或会话已过期").into_response()
    }
}

/// Cleanup expired sessions periodically
pub async fn session_gc(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(600)).await;
        let mut s = state.sessions.lock().await;
        s.retain(|_, v| v.expires > Instant::now());
    }
}
