//! Network → Multi-WAN / Policy Routing：多上行、故障切换/负载均衡与基于源地址的策略路由。

use axum::{extract::State, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    cmd,
    state::{self, AppState},
};

const KEY: &str = "mwan_config";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Wan {
    pub name: String,
    pub iface: String,
    pub gateway: String,
    pub table: u32,
    pub metric: u32,
    pub weight: u32,
    pub probe: String,
    pub enabled: bool,
}

impl Default for Wan {
    fn default() -> Self {
        Self {
            name: "wan1".into(),
            iface: String::new(),
            gateway: String::new(),
            table: 101,
            metric: 10,
            weight: 1,
            probe: "223.5.5.5".into(),
            enabled: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Policy {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub table: u32,
    pub priority: u32,
    pub enabled: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            name: "lan-via-wan1".into(),
            src: "192.168.1.0/24".into(),
            dst: String::new(),
            table: 101,
            priority: 1001,
            enabled: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct MwanConfig {
    pub enabled: bool,
    pub mode: String, // failover | balance
    pub wans: Vec<Wan>,
    pub policies: Vec<Policy>,
}

impl Default for MwanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "failover".into(),
            wans: Vec::new(),
            policies: Vec::new(),
        }
    }
}

fn load(db: &rusqlite::Connection) -> MwanConfig {
    state::get_setting(db, KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn validate(cfg: &MwanConfig) -> Result<(), String> {
    if !matches!(cfg.mode.as_str(), "failover" | "balance") {
        return Err("模式须为 failover / balance".into());
    }
    if cfg.wans.len() > 16 || cfg.policies.len() > 64 {
        return Err("WAN 或策略数量超过上限".into());
    }
    let mut tables = std::collections::HashSet::new();
    for w in &cfg.wans {
        if !cmd::safe(&w.name, 32) || !cmd::safe(&w.iface, 32) {
            return Err(format!("WAN 名称/接口非法：{}", w.name));
        }
        if !cmd::is_ip(&w.gateway) {
            return Err(format!("网关地址非法：{}", w.gateway));
        }
        if !(100..=250).contains(&w.table) {
            return Err("路由表 ID 须在 100-250（避免与系统主表冲突）".into());
        }
        if !tables.insert(w.table) {
            return Err(format!("路由表 ID {} 重复", w.table));
        }
        if w.metric > 32767 || !(1..=255).contains(&w.weight) {
            return Err("metric / weight 超出范围".into());
        }
        if !w.probe.is_empty() && !cmd::is_ip(&w.probe) && !cmd::is_domain(&w.probe) {
            return Err("探测地址非法".into());
        }
    }
    for p in &cfg.policies {
        if !(1000..=1999).contains(&p.priority) {
            return Err("策略优先级须在 1000-1999（bianwrt 保留段）".into());
        }
        if !(100..=250).contains(&p.table) {
            return Err("策略引用的路由表 ID 须在 100-250".into());
        }
        if !p.src.is_empty() && p.src != "all" && !cmd::is_ip_or_cidr(&p.src) {
            return Err(format!("源地址非法：{}", p.src));
        }
        if !p.dst.is_empty() && p.dst != "all" && !cmd::is_ip_or_cidr(&p.dst) {
            return Err(format!("目标地址非法：{}", p.dst));
        }
    }
    Ok(())
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

fn plan(cfg: &MwanConfig) -> Vec<Step> {
    let mut out = Vec::new();
    if !cfg.enabled {
        return out;
    }
    let active: Vec<&Wan> = cfg.wans.iter().filter(|w| w.enabled).collect();
    if active.is_empty() {
        return out;
    }
    for w in &active {
        let t = w.table.to_string();
        let m = w.metric.to_string();
        out.push(step(&["ip", "route", "flush", "table", &t], true, "清空自定义路由表"));
        out.push(step(
            &["ip", "route", "replace", "default", "via", &w.gateway, "dev", &w.iface,
              "table", &t, "metric", &m],
            false,
            "写入该 WAN 的独立路由表",
        ));
    }
    if cfg.mode == "balance" {
        let mut args: Vec<String> = vec!["ip".into(), "route".into(), "replace".into(), "default".into()];
        for w in &active {
            args.push("nexthop".into());
            args.push("via".into());
            args.push(w.gateway.clone());
            args.push("dev".into());
            args.push(w.iface.clone());
            args.push("weight".into());
            args.push(w.weight.to_string());
        }
        out.push(Step { args, optional: false, note: "主表多路径默认路由（按权重负载均衡）".into() });
    } else {
        // 故障切换：所有 WAN 的默认路由都进主表，靠 metric 决定优先级；链路 down 时内核自动移除。
        for w in &active {
            let m = w.metric.to_string();
            out.push(Step {
                args: vec!["ip".into(), "route".into(), "replace".into(), "default".into(),
                           "via".into(), w.gateway.clone(), "dev".into(), w.iface.clone(),
                           "metric".into(), m],
                optional: false,
                note: format!("主表默认路由：{} (metric {})", w.name, w.metric),
            });
        }
    }
    for p in cfg.policies.iter().filter(|p| p.enabled) {
        let t = p.table.to_string();
        let pref = p.priority.to_string();
        let mut args: Vec<String> = vec!["ip".into(), "rule".into(), "add".into()];
        if !p.src.is_empty() {
            args.push("from".into());
            args.push(p.src.clone());
        }
        if !p.dst.is_empty() {
            args.push("to".into());
            args.push(p.dst.clone());
        }
        args.push("lookup".into());
        args.push(t);
        args.push("pref".into());
        args.push(pref);
        // 先删同优先级旧规则，避免重复 add 失败
        let mut del = args.clone();
        del[2] = "del".into();
        out.push(Step {
            args: del,
            optional: true,
            note: "清理同优先级旧规则".into(),
        });
        out.push(Step { args, optional: false, note: format!("策略规则：{}", p.name) });
    }
    out.push(step(&["ip", "route", "flush", "cache"], true, "刷新路由缓存"));
    out
}

fn plan_json(steps: &[Step]) -> Vec<Value> {
    steps.iter().map(|s| json!({ "cmd": s.args.join(" "), "optional": s.optional, "note": s.note })).collect()
}

async fn ip_json(args: &[&str]) -> Value {
    let out = cmd::run("ip", args, 6).await;
    if out.ok {
        serde_json::from_str(&out.stdout).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

async fn execute(steps: &[Step]) -> (Vec<Value>, Option<String>) {
    let mut done = Vec::new();
    for s in steps {
        let refs: Vec<&str> = s.args[1..].iter().map(|x| x.as_str()).collect();
        let out = cmd::run(&s.args[0], &refs, 10).await;
        let ok = out.ok;
        done.push(json!({
            "cmd": s.args.join(" "), "ok": ok, "note": s.note,
            "error": if ok { "" } else { out.stderr.trim() },
        }));
        if !ok && !s.optional {
            return (done, Some(format!("{} 失败：{}", s.args.join(" "), out.stderr.trim())));
        }
    }
    (done, None)
}

pub async fn status() -> Response {
    let rules = ip_json(&["-j", "rule"]).await;
    let routes = ip_json(&["-j", "-4", "route", "show", "table", "all"]).await;
    let main = ip_json(&["-j", "-4", "route"]).await;
    let links = ip_json(&["-j", "-4", "addr"]).await;
    let ifaces: Vec<Value> = links
        .as_array()
        .map(|a| a.iter().map(|x| json!({
            "name": x["ifname"],
            "addr": x["addr_info"].as_array().and_then(|v| v.first()).map(|v| v["local"].clone()).unwrap_or(json!(null)),
        })).collect())
        .unwrap_or_default();
    cmd::ok(json!({
        "ip": cmd::which("ip"),
        "rules": rules,
        "routes": routes,
        "main": main,
        "ifaces": ifaces,
    }))
}

pub async fn config_get(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    cmd::ok(json!({
        "config": cfg,
        "plan": plan_json(&plan(&cfg)),
        "modes": [
            { "label": "故障切换 (failover)", "value": "failover" },
            { "label": "按权重负载均衡 (balance)", "value": "balance" }
        ],
    }))
}

pub async fn config_put(State(state): State<AppState>, Json(cfg): Json<MwanConfig>) -> Response {
    if let Err(e) = validate(&cfg) {
        return cmd::err(e);
    }
    let db = state.db.lock().unwrap();
    state::set_setting(&db, KEY, &serde_json::to_string(&cfg).unwrap());
    cmd::ok(json!({ "ok": true, "plan": plan_json(&plan(&cfg)) }))
}

pub async fn apply(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    if !cfg.enabled || cfg.wans.iter().all(|w| !w.enabled) {
        return cmd::err("请先启用多 WAN 并至少启用一条上行");
    }
    if !cmd::which("ip") {
        return cmd::fail("未安装 iproute2：apt install iproute2");
    }
    let steps = plan(&cfg);
    let (done, error) = execute(&steps).await;
    let script = format!(
        "#!/bin/sh\n# generated by bianwrt — do not edit\n{}\n",
        steps.iter().map(|s| s.args.join(" ")).collect::<Vec<_>>().join("\n")
    );
    let _ = std::fs::write(state.conf_dir.join("mwan-apply.sh"), &script);
    if let Some(e) = error {
        return cmd::fail(format!("{}（已执行 {} 条，可用「清除规则」回滚）", e, done.len()));
    }
    cmd::ok(json!({ "ok": true, "steps": done, "msg": "策略路由已应用（即时生效，重启后失效）" }))
}

pub async fn clear(State(state): State<AppState>) -> Response {
    let cfg = load(&state.db.lock().unwrap());
    let tables: std::collections::HashSet<u32> = cfg.wans.iter().map(|w| w.table).collect();
    let rules = ip_json(&["-j", "rule"]).await;
    let mut removed = 0usize;
    let mut steps = Vec::new();
    if let Some(list) = rules.as_array() {
        for r in list {
            let lookup = r["table"].as_str().unwrap_or("");
            let tid: Option<u32> = lookup.parse().ok();
            let hit = tid.map(|t| tables.contains(&t)).unwrap_or(false)
                || r["priority"].as_u64().map(|p| (1000..=1999).contains(&p)).unwrap_or(false) && !lookup.is_empty();
            if !hit {
                continue;
            }
            let pref = r["priority"].as_u64().map(|p| p.to_string()).unwrap_or_default();
            let src = r["src"].as_str().unwrap_or("");
            let mut args: Vec<String> = vec!["ip".into(), "rule".into(), "del".into()];
            if !src.is_empty() && src != "all" {
                args.push("from".into());
                args.push(src.into());
            }
            if !lookup.is_empty() {
                args.push("lookup".into());
                args.push(lookup.into());
            }
            if !pref.is_empty() {
                args.push("pref".into());
                args.push(pref);
            }
            let refs: Vec<&str> = args[1..].iter().map(|x| x.as_str()).collect();
            let out = cmd::run("ip", &refs, 8).await;
            steps.push(json!({ "cmd": args.join(" "), "ok": out.ok }));
            if out.ok {
                removed += 1;
            }
        }
    }
    for t in &tables {
        let ts = t.to_string();
        let out = cmd::run("ip", &["route", "flush", "table", &ts], 8).await;
        steps.push(json!({ "cmd": format!("ip route flush table {}", ts), "ok": out.ok }));
    }
    cmd::ok(json!({
        "ok": true,
        "removed_rules": removed,
        "steps": steps,
        "msg": "已清除 bianwrt 管理的策略规则与自定义路由表",
    }))
}
