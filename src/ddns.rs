//! Services → DDNS：多服务商动态域名更新、公网 IP 探测、内置后台定时 worker。

use axum::{extract::State, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::{
    cmd,
    state::{self, AppState},
};

const KEY: &str = "ddns_config";
const HISTORY_FILE: &str = "ddns-history.json";
const MASK: &str = "__BIANWRT_KEEP__";
const MAX_HISTORY: usize = 60;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct DdnsEntry {
    pub name: String,
    pub provider: String,
    pub enabled: bool,
    pub domain: String,
    pub zone_id: String,
    pub record_id: String,
    pub token: String,
    pub username: String,
    pub password: String,
    pub ip_source: String, // external | interface | manual
    pub ip_interface: String,
    pub manual_ip: String,
    pub url: String,
    pub interval: u32, // 分钟
}

impl Default for DdnsEntry {
    fn default() -> Self {
        Self {
            name: "ddns1".into(),
            provider: "duckdns".into(),
            enabled: true,
            domain: String::new(),
            zone_id: String::new(),
            record_id: String::new(),
            token: String::new(),
            username: String::new(),
            password: String::new(),
            ip_source: "external".into(),
            ip_interface: String::new(),
            manual_ip: String::new(),
            url: String::new(),
            interval: 10,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct DdnsConfig {
    pub enabled: bool,
    pub entries: Vec<DdnsEntry>,
}

fn load(db: &rusqlite::Connection) -> DdnsConfig {
    state::get_setting(db, KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(state: &AppState, cfg: &DdnsConfig) {
    let db = state.db.lock().unwrap();
    state::set_setting(&db, KEY, &serde_json::to_string(cfg).unwrap());
}

/// 返回前端时隐藏密钥；提交时 MASK 表示「保持原值」。
fn mask(cfg: &DdnsConfig) -> DdnsConfig {
    let mut c = cfg.clone();
    for e in &mut c.entries {
        if !e.token.is_empty() { e.token = MASK.into(); }
        if !e.password.is_empty() { e.password = MASK.into(); }
    }
    c
}

fn unmask(mut new: DdnsConfig, old: &DdnsConfig) -> DdnsConfig {
    for (i, e) in new.entries.iter_mut().enumerate() {
        let prev = old.entries.get(i);
        if e.token == MASK {
            e.token = prev.map(|p| p.token.clone()).unwrap_or_default();
        }
        if e.password == MASK {
            e.password = prev.map(|p| p.password.clone()).unwrap_or_default();
        }
    }
    new
}

fn providers() -> Vec<Value> {
    vec![
        json!({
            "id": "duckdns", "name": "DuckDNS", "basic_auth": false,
            "url": "https://www.duckdns.org/update?domains={domain}&token={token}&ip={ip}",
            "fields": ["domain", "token"],
            "note": "domain 填子域名（不含 .duckdns.org）"
        }),
        json!({
            "id": "noip", "name": "No-IP", "basic_auth": true,
            "url": "https://dynupdate.no-ip.com/nic/update?hostname={domain}&myip={ip}",
            "fields": ["domain", "username", "password"],
            "note": "用户名/密码用于 HTTP Basic 认证"
        }),
        json!({
            "id": "dynu", "name": "Dynu", "basic_auth": true,
            "url": "https://api.dynu.com/nic/update?hostname={domain}&myip={ip}",
            "fields": ["domain", "username", "password"],
            "note": "密码建议使用 IP Update Password"
        }),
        json!({
            "id": "cloudflare", "name": "Cloudflare", "basic_auth": false,
            "url": "https://api.cloudflare.com/client/v4/zones/{zone}/dns_records/{record}",
            "fields": ["domain", "token", "zone_id", "record_id"],
            "note": "使用 API Token + Zone ID + DNS Record ID，走 PATCH JSON"
        }),
        json!({
            "id": "oray", "name": "花生壳 (Oray)", "basic_auth": true,
            "url": "http://ddns.oray.com/ph/update?hostname={domain}&myip={ip}",
            "fields": ["domain", "username", "password"],
            "note": "使用账号与密码"
        }),
        json!({
            "id": "custom", "name": "自定义 URL", "basic_auth": false,
            "url": "", "fields": ["url"],
            "note": "支持占位符 {ip} {domain} {token} {zone} {record}"
        }),
    ]
}

fn provider_meta(id: &str) -> Option<Value> {
    providers().into_iter().find(|p| p["id"] == id)
}

fn validate(cfg: &DdnsConfig) -> Result<(), String> {
    if cfg.entries.len() > 32 {
        return Err("DDNS 条目超过上限（32）".into());
    }
    for e in &cfg.entries {
        if !cmd::safe(&e.name, 48) {
            return Err("条目名称非法".into());
        }
        let meta = provider_meta(&e.provider).ok_or("未知服务商")?;
        if !cmd::is_domain(&e.domain) {
            return Err(format!("域名非法：{}", e.domain));
        }
        if !matches!(e.ip_source.as_str(), "external" | "interface" | "manual") {
            return Err("IP 来源须为 external / interface / manual".into());
        }
        if e.ip_source == "manual" && !cmd::is_ip(&e.manual_ip) {
            return Err("手动 IP 地址非法".into());
        }
        if e.ip_source == "interface" && !cmd::safe(&e.ip_interface, 32) {
            return Err("接口名非法".into());
        }
        if !(1..=1440).contains(&e.interval) {
            return Err("更新间隔须在 1-1440 分钟".into());
        }
        for f in ["token", "password", "username", "zone_id", "record_id"] {
            let v = match f {
                "token" => &e.token,
                "password" => &e.password,
                "username" => &e.username,
                "zone_id" => &e.zone_id,
                _ => &e.record_id,
            };
            if !v.is_empty() && (v.len() > 256 || v.chars().any(|c| c.is_control() || c.is_whitespace())) {
                return Err(format!("{} 含非法字符", f));
            }
        }
        if e.provider == "cloudflare" {
            if !cmd::safe(&e.zone_id, 64) || !cmd::safe(&e.record_id, 64) {
                return Err("Cloudflare Zone/Record ID 非法".into());
            }
        } else {
            let tpl = if e.provider == "custom" {
                e.url.clone()
            } else {
                meta["url"].as_str().unwrap_or("").to_string()
            };
            if !cmd::safe_url(&tpl) {
                return Err("更新 URL 非法（须为 http/https 且不含空白）".into());
            }
        }
    }
    Ok(())
}

fn history_path(state: &AppState) -> std::path::PathBuf {
    state.conf_dir.join(HISTORY_FILE)
}

fn read_history(state: &AppState) -> Vec<Value> {
    std::fs::read_to_string(history_path(state))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default()
}

fn push_history(state: &AppState, record: Value) {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _g = LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut h = read_history(state);
    h.insert(0, record);
    h.truncate(MAX_HISTORY);
    let _ = std::fs::write(history_path(state), serde_json::to_string_pretty(&h).unwrap_or_default());
}

async fn public_ip(e: &DdnsEntry) -> Result<String, String> {
    match e.ip_source.as_str() {
        "manual" => Ok(e.manual_ip.trim().to_string()),
        "interface" => {
            let out = cmd::run("ip", &["-j", "-4", "addr", "show", "dev", &e.ip_interface], 6).await;
            let v: Value = serde_json::from_str(&out.stdout).unwrap_or(Value::Null);
            let ip = v.as_array()
                .and_then(|a| a.first())
                .and_then(|x| x["addr_info"].as_array())
                .and_then(|a| a.first())
                .and_then(|x| x["local"].as_str())
                .unwrap_or("")
                .to_string();
            if cmd::is_ip(&ip) { Ok(ip) } else { Err("接口上未找到 IPv4 地址".into()) }
        }
        _ => {
            for u in ["https://api.ipify.org", "https://ifconfig.me/ip", "https://ip.sb"] {
                let out = cmd::run("curl", &["-sS", "--max-time", "10", u], 12).await;
                let ip = out.stdout.trim().to_string();
                if out.ok && cmd::is_ip(&ip) {
                    return Ok(ip);
                }
            }
            Err("无法获取公网 IP（curl 不可用或网络不通）".into())
        }
    }
}

fn fill_template(tpl: &str, e: &DdnsEntry, ip: &str) -> String {
    tpl.replace("{ip}", ip)
        .replace("{domain}", &e.domain)
        .replace("{token}", &e.token)
        .replace("{user}", &e.username)
        .replace("{password}", &e.password)
        .replace("{username}", &e.username)
        .replace("{zone}", &e.zone_id)
        .replace("{record}", &e.record_id)
}

fn scrub(body: &str, e: &DdnsEntry) -> String {
    let mut s = body.chars().take(300).collect::<String>();
    for secret in [&e.token, &e.password] {
        if secret.len() > 3 {
            s = s.replace(secret.as_str(), "***");
        }
    }
    s
}

async fn update_one(e: &DdnsEntry) -> Value {
    let ip = match public_ip(e).await {
        Ok(v) => v,
        Err(m) => {
            return json!({ "name": e.name, "ok": false, "domain": e.domain, "ip": "", "msg": m });
        }
    };
    if !cmd::which("curl") {
        return json!({ "name": e.name, "ok": false, "domain": e.domain, "ip": ip, "msg": "未安装 curl" });
    }
    let meta = provider_meta(&e.provider).unwrap_or(json!({}));
    let basic = meta["basic_auth"].as_bool().unwrap_or(false);
    let mut args: Vec<String> = vec![
        "curl".into(), "-sS".into(), "--max-time".into(), "15".into(),
        "-w".into(), "\\n%{http_code}".into(),
    ];
    if e.provider == "cloudflare" {
        args.push("-X".into());
        args.push("PATCH".into());
        args.push("-H".into());
        args.push(format!("Authorization: Bearer {}", e.token));
        args.push("-H".into());
        args.push("Content-Type: application/json".into());
        args.push("-d".into());
        args.push(json!({
            "type": "A", "name": e.domain, "content": ip,
            "ttl": 120, "proxied": false
        }).to_string());
        args.push(format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            e.zone_id, e.record_id
        ));
    } else {
        let tpl = if e.provider == "custom" {
            e.url.clone()
        } else {
            meta["url"].as_str().unwrap_or("").to_string()
        };
        let url = fill_template(&tpl, e, &ip);
        if !cmd::safe_url(&url) {
            return json!({ "name": e.name, "ok": false, "domain": e.domain, "ip": ip, "msg": "更新 URL 含非法字符" });
        }
        if basic {
            args.push("-u".into());
            args.push(format!("{}:{}", e.username, e.password));
        }
        args.push(url);
    }
    let refs: Vec<&str> = args[1..].iter().map(|x| x.as_str()).collect();
    let out = cmd::run("curl", &refs, 20).await;
    let (body, code) = out.stdout.trim().rsplit_once('\n').unwrap_or((out.stdout.trim(), "0"));
    let ok = out.ok && code.starts_with('2');
    let mut msg = format!("HTTP {} · {}", code, scrub(body, e));
    if !out.ok {
        msg = out.stderr.trim().to_string();
    }
    json!({
        "name": e.name, "ok": ok, "domain": e.domain, "ip": ip,
        "provider": e.provider, "msg": msg,
    })
}

async fn run_all(state: &AppState, cfg: &DdnsConfig) -> Vec<Value> {
    let mut results = Vec::new();
    for e in cfg.entries.iter().filter(|x| x.enabled) {
        let r = update_one(e).await;
        let rec = json!({
            "ts": chrono::Utc::now().timestamp(),
            "name": e.name,
            "domain": e.domain,
            "provider": e.provider,
            "ok": r["ok"],
            "ip": r["ip"],
            "msg": r["msg"],
        });
        push_history(state, rec);
        results.push(r);
    }
    results
}

/// 后台定时 worker：每 60s 检查一次，按每个条目的 interval 触发更新。
pub async fn start_worker(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let cfg = load(&state.db.lock().unwrap());
        if !cfg.enabled || cfg.entries.is_empty() {
            continue;
        }
        let history = read_history(&state);
        let now = chrono::Utc::now().timestamp();
        let due: Vec<DdnsEntry> = cfg
            .entries
            .iter()
            .filter(|e| {
                if !e.enabled {
                    return false;
                }
                let last = history
                    .iter()
                    .find(|h| h["name"] == e.name)
                    .and_then(|h| h["ts"].as_i64())
                    .unwrap_or(0);
                now - last >= (e.interval as i64) * 60
            })
            .cloned()
            .collect();
        if due.is_empty() {
            continue;
        }
        let sub = DdnsConfig { enabled: true, entries: due };
        let _ = run_all(&state, &sub).await;
    }
}

pub async fn providers_handler() -> Response {
    cmd::ok(json!({ "items": providers(), "curl": cmd::which("curl") }))
}

pub async fn config_get(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    cmd::ok(json!({ "config": mask(&cfg), "providers": providers() }))
}

pub async fn config_put(State(state): State<AppState>, Json(input): Json<DdnsConfig>) -> Response {
    let old = load(&state.db.lock().unwrap());
    let cfg = unmask(input, &old);
    if let Err(e) = validate(&cfg) {
        return cmd::err(e);
    }
    save(&state, &cfg);
    cmd::ok(json!({ "ok": true, "config": mask(&cfg) }))
}

pub async fn update(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    if cfg.entries.is_empty() {
        return cmd::err("请先添加 DDNS 条目");
    }
    let results = run_all(&state, &cfg).await;
    cmd::ok(json!({ "ok": true, "results": results }))
}

pub async fn history(State(state): State<AppState>) -> Response {
    cmd::ok(json!({ "items": read_history(&state), "limit": MAX_HISTORY }))
}

pub async fn apply(State(state): State<AppState>) -> Response {
    let mut cfg = load(&state.db.lock().unwrap());
    if let Err(e) = validate(&cfg) {
        return cmd::err(e);
    }
    cfg.enabled = true;
    save(&state, &cfg);
    let results = run_all(&state, &cfg).await;
    cmd::ok(json!({
        "ok": true,
        "results": results,
        "msg": "已启用后台定时更新（每 60s 检查一次，按各条目间隔执行）",
    }))
}

pub async fn clear(State(state): State<AppState>) -> Response {
    let _ = std::fs::write(history_path(&state), "[]");
    cmd::ok(json!({ "ok": true, "msg": "更新历史已清空" }))
}
