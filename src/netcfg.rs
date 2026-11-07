//! Network → Interfaces / Static Routes：可写的接口与路由管理。
//! 即时生效使用 `ip` 命令；持久化写入 /etc/network/interfaces.d 且需显式确认后应用。

use axum::{extract::Path, response::Response, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::cmd;

const PERSIST_DIR: &str = "/etc/network/interfaces.d";

fn persist_path(name: &str) -> Option<PathBuf> {
    if !cmd::safe(name, 64) || name.contains("..") {
        return None;
    }
    Some(PathBuf::from(PERSIST_DIR).join(format!("bianwrt-{}.conf", name)))
}

async fn ip_json(args: &[&str]) -> Value {
    let out = cmd::run("ip", args, 5).await;
    if out.ok {
        serde_json::from_str(&out.stdout).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

pub async fn routes() -> Response {
    let v4 = ip_json(&["-j", "-4", "route"]).await;
    let v6 = ip_json(&["-j", "-6", "route"]).await;
    let normalize = |v: &Value| -> Vec<Value> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        json!({
                            "dst": r["dst"],
                            "via": r["gateway"],
                            "dev": r["dev"],
                            "protocol": r["protocol"],
                            "metric": r["metric"],
                            "table": r["table"],
                            "prefsrc": r["prefsrc"],
                            "type": r["type"],
                            "raw": r,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    cmd::ok(json!({
        "v4": normalize(&v4),
        "v6": normalize(&v6),
        "raw": { "v4": v4, "v6": v6 },
    }))
}

#[derive(Deserialize)]
pub struct RouteReq {
    pub dest: String,
    pub family: Option<String>,
    pub via: Option<String>,
    pub dev: Option<String>,
    pub metric: Option<u32>,
    pub table: Option<String>,
}

fn build_route_args(req: &RouteReq, action: &str) -> Result<Vec<String>, String> {
    let dest = req.dest.trim();
    if dest != "default" && !cmd::is_ip_or_cidr(dest) {
        return Err("目标网段非法（需为 IP/CIDR 或 default）".into());
    }
    let fam = req.family.clone().unwrap_or_else(|| "4".into());
    if fam != "4" && fam != "6" {
        return Err("地址族须为 4 或 6".into());
    }
    let mut a = vec![format!("-{}", fam), "route".into(), action.into(), dest.into()];
    if let Some(v) = &req.via {
        if !cmd::is_ip(v) {
            return Err("下一跳地址非法".into());
        }
        a.push("via".into());
        a.push(v.clone());
    }
    if let Some(d) = &req.dev {
        if !cmd::safe(d, 32) {
            return Err("出接口名称非法".into());
        }
        a.push("dev".into());
        a.push(d.clone());
    }
    if req.via.is_none() && req.dev.is_none() {
        return Err("必须指定下一跳 via 或出接口 dev".into());
    }
    if let Some(m) = req.metric {
        a.push("metric".into());
        a.push(m.to_string());
    }
    if let Some(t) = &req.table {
        if !cmd::safe(t, 32) {
            return Err("路由表名称非法".into());
        }
        a.push("table".into());
        a.push(t.clone());
    }
    Ok(a)
}

async fn route_action(req: RouteReq, action: &str) -> Response {
    let args = match build_route_args(&req, action) {
        Ok(a) => a,
        Err(e) => return cmd::err(e),
    };
    let display = format!("ip {}", args.join(" "));
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = cmd::run("ip", &refs, 8).await;
    if !out.ok {
        return cmd::fail(format!("{} 失败：{}", display, out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true, "cmd": display, "output": out.stdout.trim() }))
}

pub async fn route_add(Json(req): Json<RouteReq>) -> Response {
    route_action(req, "add").await
}

pub async fn route_del(Json(req): Json<RouteReq>) -> Response {
    route_action(req, "del").await
}

pub async fn interfaces() -> Response {
    let addrs = ip_json(&["-j", "-d", "addr"]).await;
    let links = ip_json(&["-j", "link"]).await;
    let mut items = Vec::new();
    if let Some(a) = addrs.as_array() {
        for x in a {
            let name = x["ifname"].as_str().unwrap_or("").to_string();
            let link = links
                .as_array()
                .and_then(|l| l.iter().find(|v| v["ifname"] == name));
            let flags = x["flags"]
                .as_array()
                .map(|f| f.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            items.push(json!({
                "name": name,
                "up": flags.contains(&"UP"),
                "mac": link.map(|l| l["address"].clone()).unwrap_or(json!(null)),
                "mtu": link.map(|l| l["mtu"].clone()).unwrap_or(json!(null)),
                "state": link.map(|l| l["operstate"].clone()).unwrap_or(json!(null)),
                "type": link.map(|l| l["link_type"].clone()).unwrap_or(json!(null)),
                "addrs": x["addr_info"],
            }));
        }
    }
    cmd::ok(json!({ "items": items }))
}

#[derive(Deserialize)]
pub struct LinkReq {
    pub name: String,
    pub up: bool,
}

pub async fn link(Json(req): Json<LinkReq>) -> Response {
    if !cmd::safe(&req.name, 32) {
        return cmd::err("接口名非法");
    }
    if req.name == "lo" && !req.up {
        return cmd::err("禁止停用回环接口 lo");
    }
    let op = if req.up { "up" } else { "down" };
    let out = cmd::run("ip", &["link", "set", &req.name, op], 8).await;
    if !out.ok {
        return cmd::fail(format!("设置失败：{}", out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct AddrReq {
    pub name: String,
    pub op: String,
    pub addr: String,
}

pub async fn addr(Json(req): Json<AddrReq>) -> Response {
    if !cmd::safe(&req.name, 32) {
        return cmd::err("接口名非法");
    }
    if !matches!(req.op.as_str(), "add" | "del") {
        return cmd::err("操作须为 add 或 del");
    }
    if !cmd::is_ip_or_cidr(&req.addr) {
        return cmd::err("地址须为 IP/CIDR（如 192.168.1.1/24）");
    }
    let out = cmd::run(
        "ip",
        &["addr", &req.op, &req.addr, "dev", &req.name],
        8,
    )
    .await;
    if !out.ok {
        return cmd::fail(format!("设置失败：{}", out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct GenReq {
    pub name: String,
    pub mode: String,
    pub address: Option<String>,
    pub gateway: Option<String>,
    pub dns: Option<String>,
    pub mtu: Option<u32>,
}

pub async fn generate(Json(req): Json<GenReq>) -> Response {
    if !cmd::safe(&req.name, 32) {
        return cmd::err("接口名非法");
    }
    if !matches!(req.mode.as_str(), "dhcp" | "static" | "manual") {
        return cmd::err("模式须为 dhcp / static / manual");
    }
    let mut s = format!("# generated by bianwrt — do not edit\nauto {}\niface {} inet {}\n", req.name, req.name, req.mode);
    if req.mode == "static" {
        let a = req.address.clone().unwrap_or_default();
        if !cmd::is_ip_or_cidr(&a) {
            return cmd::err("静态地址须为 IP/CIDR");
        }
        s.push_str(&format!("    address {}\n", a));
        if let Some(g) = &req.gateway {
            if !cmd::is_ip(g) {
                return cmd::err("网关地址非法");
            }
            s.push_str(&format!("    gateway {}\n", g));
        }
        if let Some(d) = &req.dns {
            for x in d.split(|c: char| c == ',' || c.is_whitespace()).filter(|x| !x.is_empty()) {
                if !cmd::is_ip(x) {
                    return cmd::err(format!("DNS 地址非法: {}", x));
                }
            }
            s.push_str(&format!("    dns-nameservers {}\n", d.replace(',', " ")));
        }
    }
    if let Some(m) = req.mtu {
        if !(576..=9000).contains(&m) {
            return cmd::err("MTU 须在 576-9000 之间");
        }
        s.push_str(&format!("    mtu {}\n", m));
    }
    cmd::ok(json!({ "config": s, "name": req.name }))
}

pub async fn persist_list() -> Response {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(PERSIST_DIR) {
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("bianwrt-") && n.ends_with(".conf") {
                items.push(json!({
                    "file": n,
                    "name": n.trim_start_matches("bianwrt-").trim_end_matches(".conf"),
                    "content": std::fs::read_to_string(&p).unwrap_or_default(),
                }));
            }
        }
    }
    items.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    cmd::ok(json!({
        "items": items,
        "dir": PERSIST_DIR,
        "exists": std::path::Path::new(PERSIST_DIR).is_dir(),
        "netplan": std::path::Path::new("/etc/netplan").is_dir(),
        "ifupdown2": cmd::which("ifreload"),
    }))
}

#[derive(Deserialize)]
pub struct SaveReq {
    pub name: String,
    pub content: String,
}

pub async fn persist_put(Json(req): Json<SaveReq>) -> Response {
    let Some(path) = persist_path(&req.name) else {
        return cmd::err("配置名非法");
    };
    if req.content.len() > 8192 || req.content.contains('\0') {
        return cmd::err("配置内容过长或含非法字符");
    }
    if std::fs::create_dir_all(PERSIST_DIR).is_err() {
        return cmd::fail("无法创建 /etc/network/interfaces.d（需要 root）");
    }
    if std::fs::write(&path, &req.content).is_err() {
        return cmd::fail("写入配置失败（需要 root）");
    }
    cmd::ok(json!({ "ok": true, "path": path.display().to_string() }))
}

pub async fn persist_del(Path(name): Path<String>) -> Response {
    let Some(path) = persist_path(&name) else {
        return cmd::err("配置名非法");
    };
    match std::fs::remove_file(&path) {
        Ok(_) => cmd::ok(json!({ "ok": true })),
        Err(e) => cmd::fail(format!("删除失败：{}", e)),
    }
}

#[derive(Deserialize)]
pub struct ApplyReq {
    pub confirm: bool,
}

pub async fn persist_apply(Json(req): Json<ApplyReq>) -> Response {
    if !req.confirm {
        return cmd::err("请确认后再应用（会短暂中断网络）");
    }
    let (tool, args): (&str, Vec<&str>) = if cmd::which("ifreload") {
        ("ifreload", vec!["-a"])
    } else {
        ("systemctl", vec!["restart", "networking.service"])
    };
    let out = cmd::run(tool, &args, 30).await;
    if !out.ok {
        return cmd::fail(format!("{} 执行失败：{}", tool, out.stderr.trim()));
    }
    cmd::ok(json!({ "ok": true, "tool": tool }))
}
