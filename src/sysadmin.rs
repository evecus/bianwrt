//! System → System / Administration：主机名、时区、NTP、密码、SSH 公钥、重启关机。

use axum::{response::Response, Json};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use crate::{auth, cmd, state};

fn home_ssh_keys() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".ssh").join("authorized_keys")
}

fn read_trim(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub async fn info() -> Response {
    let tz = {
        let o = cmd::run("timedatectl", &["show", "-p", "Timezone", "--value"], 5).await;
        if o.ok && !o.stdout.trim().is_empty() {
            o.stdout.trim().to_string()
        } else {
            read_trim("/etc/timezone")
        }
    };
    let ntp = cmd::run("timedatectl", &["show", "-p", "NTP", "--value"], 5).await;
    let synced = cmd::run("timedatectl", &["show", "-p", "NTPSynchronized", "--value"], 5).await;
    let keys: Vec<String> = std::fs::read_to_string(home_ssh_keys())
        .unwrap_or_default()
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let uid = cmd::run("id", &["-u"], 3).await;
    cmd::ok(json!({
        "hostname": read_trim("/etc/hostname"),
        "timezone": tz,
        "ntp": ntp.stdout.trim(),
        "ntp_synchronized": synced.stdout.trim(),
        "localtime": chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string(),
        "ssh_keys": keys,
        "ssh_keys_path": home_ssh_keys().display().to_string(),
        "root": uid.stdout.trim() == "0",
        "systemd": cmd::which("systemctl"),
    }))
}

#[derive(Deserialize)]
pub struct HostnameReq {
    pub hostname: String,
}

pub async fn hostname(Json(req): Json<HostnameReq>) -> Response {
    let h = req.hostname.trim();
    if h.is_empty()
        || h.len() > 63
        || h.starts_with('-')
        || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return cmd::err("主机名只能包含字母、数字和连字符，且长度不超过 63");
    }
    let out = cmd::run("hostnamectl", &["set-hostname", h], 8).await;
    if !out.ok {
        if std::fs::write("/etc/hostname", format!("{}\n", h)).is_err() {
            return cmd::fail(format!("设置失败: {}", out.stderr.trim()));
        }
    }
    cmd::ok(json!({ "ok": true, "hostname": h }))
}

#[derive(Deserialize)]
pub struct TimezoneReq {
    pub timezone: String,
}

pub async fn timezone(Json(req): Json<TimezoneReq>) -> Response {
    let tz = req.timezone.trim();
    if !cmd::safe(tz, 64) || tz.contains("..") {
        return cmd::err("时区名称非法");
    }
    let out = cmd::run("timedatectl", &["set-timezone", tz], 8).await;
    if !out.ok {
        return cmd::fail(format!("设置时区失败: {}", out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true, "timezone": tz }))
}

pub async fn timezones() -> Response {
    let out = cmd::run("timedatectl", &["list-timezones"], 8).await;
    let items: Vec<String> = out.stdout.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    cmd::ok(json!({ "items": items }))
}

#[derive(Deserialize)]
pub struct NtpReq {
    pub enabled: bool,
}

pub async fn ntp(Json(req): Json<NtpReq>) -> Response {
    let v = if req.enabled { "1" } else { "0" };
    let out = cmd::run("timedatectl", &["set-ntp", v], 8).await;
    if !out.ok {
        return cmd::fail(format!("设置 NTP 失败: {}", out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct PasswordReq {
    pub old: String,
    pub new: String,
}

pub async fn password(
    axum::extract::State(state): axum::extract::State<state::AppState>,
    Json(req): Json<PasswordReq>,
) -> Response {
    let Some(mut admin) = auth::get_admin(&state) else {
        return cmd::err("尚未初始化管理员");
    };
    if admin.hash != auth::hash_password(&req.old, &admin.salt) {
        return cmd::err("原密码不正确");
    }
    if req.new.len() < 6 || req.new.len() > 128 {
        return cmd::err("新密码长度需为 6-128");
    }
    let salt = auth::gen_token();
    admin.hash = auth::hash_password(&req.new, &salt);
    admin.salt = salt;
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES('admin',?1)",
        [serde_json::to_string(&admin).unwrap()],
    )
    .ok();
    cmd::ok(json!({ "ok": true, "msg": "密码已更新，当前会话保持有效" }))
}

#[derive(Deserialize)]
pub struct KeysReq {
    pub lines: Vec<String>,
}

pub async fn ssh_keys_put(Json(req): Json<KeysReq>) -> Response {
    if req.lines.len() > 100 {
        return cmd::err("公钥数量超过上限（100）");
    }
    for l in &req.lines {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.len() > 2048
            || t.chars().any(|c| c == '\n' || c == '\r')
            || !(t.starts_with("ssh-") || t.starts_with("ecdsa-") || t.starts_with("sk-"))
        {
            return cmd::err("公钥格式非法（需以 ssh- / ecdsa- / sk- 开头）");
        }
    }
    let path = home_ssh_keys();
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return cmd::fail("创建 ~/.ssh 失败");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let mut content = req.lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join("\n");
    content.push('\n');
    if std::fs::write(&path, &content).is_err() {
        return cmd::fail("写入 authorized_keys 失败");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    cmd::ok(json!({ "ok": true, "path": path.display().to_string() }))
}

#[derive(Deserialize)]
pub struct PowerReq {
    pub op: String,
}

pub async fn power(Json(req): Json<PowerReq>) -> Response {
    if !matches!(req.op.as_str(), "reboot" | "poweroff") {
        return cmd::err("不支持的电源操作");
    }
    let op = req.op.clone();
    let op_bg = op.clone();
    // 延迟执行，保证 HTTP 响应能先返回给前端
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let _ = cmd::run("systemctl", &[&op_bg], 10).await;
    });
    cmd::ok(json!({ "ok": true, "msg": format!("系统即将 {}，请稍后重新连接", op) }))
}
