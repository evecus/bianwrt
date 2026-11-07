//! Network → QoS / SQM：基于 tc + cake/ifb 的双向下行/上行整形，配置预览与应用。

use axum::{extract::State, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    cmd,
    state::{self, AppState},
};

const KEY: &str = "qos_config";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct QosIface {
    pub iface: String,
    pub enabled: bool,
    pub download: u32, // kbps
    pub upload: u32,   // kbps
    pub qdisc: String, // cake | fq_codel | htb
    pub overhead: u32,
    pub diffserv: String,
}

impl Default for QosIface {
    fn default() -> Self {
        Self {
            iface: String::new(),
            enabled: true,
            download: 100_000,
            upload: 30_000,
            qdisc: "cake".into(),
            overhead: 34,
            diffserv: "diffserv4".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct QosConfig {
    pub enabled: bool,
    pub ifaces: Vec<QosIface>,
}

fn load(db: &rusqlite::Connection) -> QosConfig {
    state::get_setting(db, KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn validate(cfg: &QosConfig) -> Result<(), String> {
    if cfg.ifaces.len() > 32 {
        return Err("接口规则超过上限（32）".into());
    }
    for i in &cfg.ifaces {
        if !cmd::safe(&i.iface, 32) {
            return Err(format!("接口名非法：{}", i.iface));
        }
        if !matches!(i.qdisc.as_str(), "cake" | "fq_codel" | "htb") {
            return Err("队列规则须为 cake / fq_codel / htb".into());
        }
        if !matches!(i.diffserv.as_str(), "diffserv3" | "diffserv4" | "diffserv8" | "besteffort") {
            return Err("Diffserv 模式非法".into());
        }
        if i.download > 10_000_000 || i.upload > 10_000_000 {
            return Err("带宽超过上限（10,000,000 kbps）".into());
        }
        if i.overhead > 200 {
            return Err("overhead 须在 0-200".into());
        }
    }
    Ok(())
}

fn ifb_name(iface: &str) -> String {
    let short: String = iface.chars().take(9).collect();
    format!("ifb4{}", short)
}

struct Step {
    args: Vec<String>,
    optional: bool,
    note: String,
}

fn step(args: &[&str], optional: bool, note: &str) -> Step {
    Step {
        args: args.iter().map(|s| s.to_string()).collect(),
        optional,
        note: note.into(),
    }
}

/// 生成要执行的命令序列：出口整形 + IFB 入口重定向整形。
fn plan(cfg: &QosConfig) -> Vec<Step> {
    let mut out = Vec::new();
    if !cfg.enabled {
        return out;
    }
    for i in cfg.ifaces.iter().filter(|x| x.enabled && x.upload + x.download > 0) {
        let ov = i.overhead.to_string();
        let up = format!("{}kbit", i.upload);
        let down = format!("{}kbit", i.download);
        let ifb = ifb_name(&i.iface);

        out.push(step(&["tc", "qdisc", "del", "dev", &i.iface, "root"], true, "清理旧出口队列"));
        match i.qdisc.as_str() {
            "cake" => out.push(step(
                &["tc", "qdisc", "replace", "dev", &i.iface, "root", "cake",
                  "bandwidth", &up, i.diffserv.as_str(), "overhead", &ov],
                false,
                "出口 cake 整形",
            )),
            "fq_codel" => out.push(step(
                &["tc", "qdisc", "replace", "dev", &i.iface, "root", "fq_codel",
                  "target", "5ms", "interval", "100ms", "limit", "10240"],
                false,
                "出口 fq_codel（仅队列管理，不限速）",
            )),
            _ => {
                out.push(step(
                    &["tc", "qdisc", "replace", "dev", &i.iface, "root", "htb", "default", "1"],
                    false,
                    "出口 htb 根队列",
                ));
                out.push(step(
                    &["tc", "class", "replace", "dev", &i.iface, "parent", "1:",
                      "classid", "1:1", "htb", "rate", &up, "ceil", &up],
                    false,
                    "出口 htb 限速类",
                ));
            }
        }

        if i.download > 0 {
            out.push(step(&["modprobe", "ifb", "numifbs=1"], true, "加载 ifb 内核模块"));
            out.push(step(&["ip", "link", "add", "name", &ifb, "type", "ifb"], true, "创建 IFB 设备"));
            out.push(step(&["ip", "link", "set", &ifb, "up"], false, "启用 IFB 设备"));
            out.push(step(&["tc", "qdisc", "del", "dev", &i.iface, "ingress"], true, "清理旧 ingress 队列"));
            out.push(step(&["tc", "qdisc", "replace", "dev", &i.iface, "ingress"], false, "建立 ingress 钩子"));
            out.push(step(
                &["tc", "qdisc", "replace", "dev", &ifb, "root", "cake",
                  "bandwidth", &down, "ingress", i.diffserv.as_str(), "overhead", &ov],
                false,
                "入口 cake 整形",
            ));
            out.push(step(
                &["tc", "filter", "replace", "dev", &i.iface, "parent", "ffff:",
                  "protocol", "all", "prio", "1", "u32", "match", "u32", "0", "0",
                  "action", "mirred", "egress", "redirect", "dev", &ifb],
                false,
                "把入口流量重定向到 IFB",
            ));
        }
    }
    out
}

fn clear_plan(cfg: &QosConfig) -> Vec<Step> {
    let mut out = Vec::new();
    for i in &cfg.ifaces {
        let ifb = ifb_name(&i.iface);
        out.push(step(&["tc", "qdisc", "del", "dev", &i.iface, "root"], true, "移除出口队列"));
        out.push(step(&["tc", "qdisc", "del", "dev", &i.iface, "ingress"], true, "移除 ingress 队列"));
        out.push(step(&["ip", "link", "del", &ifb], true, "删除 IFB 设备"));
    }
    out
}

fn plan_json(steps: &[Step]) -> Vec<serde_json::Value> {
    steps
        .iter()
        .map(|s| json!({ "cmd": s.args.join(" "), "optional": s.optional, "note": s.note }))
        .collect()
}

async fn execute(steps: &[Step]) -> (Vec<serde_json::Value>, Option<String>) {
    let mut done = Vec::new();
    for s in steps {
        let program = &s.args[0];
        let refs: Vec<&str> = s.args[1..].iter().map(|x| x.as_str()).collect();
        let out = cmd::run(program, &refs, 15).await;
        let ok = out.ok;
        done.push(json!({
            "cmd": s.args.join(" "),
            "ok": ok,
            "note": s.note,
            "error": if ok { "" } else { out.stderr.trim() },
        }));
        if !ok && !s.optional {
            return (done, Some(format!("{} 失败：{}", s.args.join(" "), out.stderr.trim())));
        }
    }
    (done, None)
}

fn parse_qdisc(raw: &str) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    let mut last_iface = String::new();
    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("qdisc ") {
            let f: Vec<&str> = rest.split_whitespace().collect();
            if f.len() >= 2 {
                last_iface = f.last().unwrap_or(&"").to_string();
                items.push(json!({
                    "kind": f[0],
                    "handle": f[1],
                    "dev": if f.len() > 3 && f[2] == "dev" { f[3] } else { "" },
                    "raw": t,
                }));
            }
        } else if t.starts_with("qdisc ") == false && !t.is_empty() && !last_iface.is_empty() {
            // 统计行，附到最近一条
            if let Some(last) = items.last_mut() {
                let extra = last["stats"].as_str().unwrap_or("").to_string();
                last["stats"] = json!(format!("{}\n{}", extra, t).trim().to_string());
            }
        }
    }
    items
}

pub async fn status() -> Response {
    let tc = cmd::which("tc");
    let mut raw = String::new();
    let mut stats = String::new();
    if tc {
        raw = cmd::run("tc", &["qdisc", "show"], 8).await.stdout;
        stats = cmd::run("tc", &["-s", "qdisc", "show"], 8).await.stdout;
    }
    let cake = std::path::Path::new("/sys/module/sch_cake").exists()
        || std::fs::read_to_string("/proc/modules")
            .map(|s| s.contains("sch_cake"))
            .unwrap_or(false);
    cmd::ok(json!({
        "tc": tc,
        "cake": cake,
        "ifb": std::path::Path::new("/sys/module/ifb").exists(),
        "qdiscs": parse_qdisc(&raw),
        "raw": raw,
        "stats": cmd::tail_lines(&stats, 120).join("\n"),
        "msg": if !tc { "未安装 tc：apt install iproute2" } else if !cake { "内核缺少 sch_cake 模块，建议 apt install linux-image-generic / 使用较新内核" } else { "" },
    }))
}

pub async fn config_get(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    cmd::ok(json!({
        "config": cfg,
        "plan": plan_json(&plan(&cfg)),
        "qdiscs": ["cake", "fq_codel", "htb"],
        "diffservs": ["diffserv3", "diffserv4", "diffserv8", "besteffort"],
    }))
}

pub async fn config_put(State(state): State<AppState>, Json(cfg): Json<QosConfig>) -> Response {
    if let Err(e) = validate(&cfg) {
        return cmd::err(e);
    }
    let db = state.db.lock().unwrap();
    state::set_setting(&db, KEY, &serde_json::to_string(&cfg).unwrap());
    cmd::ok(json!({ "ok": true, "plan": plan_json(&plan(&cfg)) }))
}

pub async fn apply(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    if !cfg.enabled || cfg.ifaces.iter().all(|x| !x.enabled) {
        return cmd::err("请先启用 QoS 并至少启用一条接口规则");
    }
    if !cmd::which("tc") {
        return cmd::fail("未安装 tc：apt install iproute2");
    }
    let steps = plan(&cfg);
    let (done, error) = execute(&steps).await;
    // 把命令序列落到 conf_dir，便于备份与排障
    let script = format!(
        "#!/bin/sh\n# generated by bianwrt — do not edit\n{}\n",
        steps.iter().map(|s| s.args.join(" ")).collect::<Vec<_>>().join("\n")
    );
    let _ = std::fs::write(state.conf_dir.join("qos-apply.sh"), &script);
    if let Some(e) = error {
        return cmd::fail(format!("{}（已执行 {} 条命令，部分步骤已回滚失败）", e, done.len()));
    }
    cmd::ok(json!({ "ok": true, "steps": done, "msg": "QoS 规则已应用（重启后失效，请重新应用或持久化）" }))
}

pub async fn clear(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    if !cmd::which("tc") {
        return cmd::fail("未安装 tc：apt install iproute2");
    }
    let steps = clear_plan(&cfg);
    let (done, _) = execute(&steps).await;
    cmd::ok(json!({ "ok": true, "steps": done, "msg": "已清理 bianwrt 配置的队列与 IFB 设备" }))
}
