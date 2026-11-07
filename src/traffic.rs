//! Status → Traffic：基于 /proc/net/dev 的采样器，提供实时速率与历史趋势数据。

use axum::{extract::Query, response::Response};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::cmd;

const INTERVAL: u64 = 5;      // 采样间隔（秒）
const MAX_POINTS: usize = 720; // 720 * 5s = 1 小时
const FILE: &str = "traffic-history.json";

#[derive(Clone)]
struct Point {
    ts: i64,
    rx: u64,
    tx: u64,
}

struct IfaceHist {
    rx_total: u64,
    tx_total: u64,
    rx_bps: u64,
    tx_bps: u64,
    points: VecDeque<Point>,
}

struct History {
    ifaces: HashMap<String, IfaceHist>,
    prev: HashMap<String, (u64, u64)>,
    prev_ts: i64,
}

impl History {
    fn new() -> Self {
        Self {
            ifaces: HashMap::new(),
            prev: HashMap::new(),
            prev_ts: 0,
        }
    }
}

static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();

fn store() -> &'static Mutex<History> {
    HISTORY.get_or_init(|| Mutex::new(History::new()))
}

/// 读取 /proc/net/dev 的累计字节数（rx, tx）。
fn counters() -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    let text = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 16 {
            continue;
        }
        let rx = f[0].parse::<u64>().unwrap_or(0);
        let tx = f[8].parse::<u64>().unwrap_or(0);
        out.insert(name.trim().to_string(), (rx, tx));
    }
    out
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn sample(h: &mut History) {
    let now = now_ts();
    let dt = (now - h.prev_ts).clamp(1, 3600) as u64;
    h.prev_ts = now;
    let cur = counters();
    for (name, (rx, tx)) in &cur {
        let (prx, ptx) = h.prev.get(name).copied().unwrap_or((*rx, *tx));
        let drx = rx.saturating_sub(prx);
        let dtx = tx.saturating_sub(ptx);
        let e = h.ifaces.entry(name.clone()).or_insert_with(|| IfaceHist {
            rx_total: *rx,
            tx_total: *tx,
            rx_bps: 0,
            tx_bps: 0,
            points: VecDeque::new(),
        });
        e.rx_total = *rx;
        e.tx_total = *tx;
        e.rx_bps = drx / dt;
        e.tx_bps = dtx / dt;
        e.points.push_back(Point { ts: now, rx: e.rx_bps, tx: e.tx_bps });
        while e.points.len() > MAX_POINTS {
            e.points.pop_front();
        }
        h.prev.insert(name.clone(), (*rx, *tx));
    }
    let stale: Vec<String> = h.prev.keys().filter(|k| !cur.contains_key(*k)).cloned().collect();
    for k in stale {
        h.prev.remove(&k);
    }
}

fn persistence_path(dir: &PathBuf) -> PathBuf {
    dir.join(FILE)
}

fn load_disk(dir: &PathBuf) {
    let Ok(text) = std::fs::read_to_string(persistence_path(dir)) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return };
    let Some(map) = v["ifaces"].as_object() else { return };
    let mut h = store().lock().unwrap();
    for (name, arr) in map {
        let Some(items) = arr.as_array() else { continue };
        let mut points = VecDeque::new();
        for p in items.iter().rev().take(MAX_POINTS) {
            let Some(a) = p.as_array() else { continue };
            if a.len() < 3 { continue; }
            points.push_front(Point {
                ts: a[0].as_i64().unwrap_or(0),
                rx: a[1].as_u64().unwrap_or(0),
                tx: a[2].as_u64().unwrap_or(0),
            });
        }
        let e = h.ifaces.entry(name.clone()).or_insert_with(|| IfaceHist {
            rx_total: 0, tx_total: 0, rx_bps: 0, tx_bps: 0, points: VecDeque::new(),
        });
        e.points = points;
    }
}

fn save_disk(dir: &PathBuf) {
    let h = store().lock().unwrap();
    let mut map = serde_json::Map::new();
    for (name, e) in &h.ifaces {
        let arr: Vec<Value> = e.points.iter()
            .map(|p| json!([p.ts, p.rx, p.tx]))
            .collect();
        map.insert(name.clone(), Value::Array(arr));
    }
    let _ = std::fs::write(
        persistence_path(dir),
        serde_json::to_string(&json!({ "ifaces": map })).unwrap_or_default(),
    );
}

/// 后台采样任务：每 5s 采样一次，每 12 次（1 分钟）落盘一次。
pub async fn start_sampler(dir: PathBuf) {
    load_disk(&dir);
    {
        let mut h = store().lock().unwrap();
        h.prev = counters();
        h.prev_ts = now_ts();
    }
    let mut ticks: u64 = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(INTERVAL)).await;
        {
            let mut h = store().lock().unwrap();
            sample(&mut h);
        }
        ticks += 1;
        if ticks % 12 == 0 {
            save_disk(&dir);
        }
    }
}

fn selected_iface(q: &HashMap<String, String>) -> Option<String> {
    q.get("iface").cloned()
}

pub async fn interfaces() -> Response {
    let h = store().lock().unwrap();
    let mut items: Vec<Value> = h
        .ifaces
        .iter()
        .map(|(name, e)| json!({
            "name": name,
            "rx_total": e.rx_total,
            "tx_total": e.tx_total,
            "rx_bps": e.rx_bps,
            "tx_bps": e.tx_bps,
        }))
        .collect();
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    cmd::ok(json!({
        "items": items,
        "interval": INTERVAL,
        "max_points": MAX_POINTS,
    }))
}

/// 当前实时速率与累计流量，用于卡片与轮询追加图表数据。
pub async fn live(Query(q): Query<HashMap<String, String>>) -> Response {
    let h = store().lock().unwrap();
    let iface = selected_iface(&q);
    let picked: Vec<(&String, &IfaceHist)> = match &iface {
        Some(n) => h.ifaces.iter().filter(|(k, _)| k.as_str() == n.as_str()).collect(),
        None => h.ifaces.iter().filter(|(k, _)| k.as_str() != "lo").collect(),
    };
    let rx_bps = picked.iter().map(|(_, e)| e.rx_bps).sum::<u64>();
    let tx_bps = picked.iter().map(|(_, e)| e.tx_bps).sum::<u64>();
    cmd::ok(json!({
        "ts": now_ts(),
        "iface": q.get("iface").cloned().unwrap_or_else(|| "__all__".into()),
        "rx_bps": rx_bps,
        "tx_bps": tx_bps,
        "rx_total": picked.iter().map(|(_, e)| e.rx_total).sum::<u64>(),
        "tx_total": picked.iter().map(|(_, e)| e.tx_total).sum::<u64>(),
    }))
}

pub async fn history(Query(q): Query<HashMap<String, String>>) -> Response {
    let h = store().lock().unwrap();
    let iface = selected_iface(&q);
    let mut all_ifaces: Vec<String> = h.ifaces.keys().cloned().collect();
    all_ifaces.sort();
    let limit: usize = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(MAX_POINTS).min(MAX_POINTS);

    // 多接口按时间戳对齐求和；单接口直接取。
    let mut merged: HashMap<i64, (u64, u64)> = HashMap::new();
    let chosen: Vec<&String> = match &iface {
        Some(n) => h.ifaces.keys().filter(|k| k.as_str() == n.as_str()).collect(),
        None => h.ifaces.keys().filter(|k| k.as_str() != "lo").collect(),
    };
    for n in &chosen {
        if let Some(e) = h.ifaces.get(*n) {
            for p in e.points.iter().rev().take(limit) {
                let v = merged.entry(p.ts).or_insert((0, 0));
                v.0 += p.rx;
                v.1 += p.tx;
            }
        }
    }
    let mut ts: Vec<i64> = merged.keys().copied().collect();
    ts.sort();
    let points: Vec<Value> = ts.iter()
        .map(|t| {
            let (rx, tx) = merged.get(t).copied().unwrap_or((0, 0));
            json!({ "ts": t, "rx": rx, "tx": tx })
        })
        .collect();
    let peak_rx = points.iter().filter_map(|p| p["rx"].as_u64()).max().unwrap_or(0);
    let peak_tx = points.iter().filter_map(|p| p["tx"].as_u64()).max().unwrap_or(0);
    cmd::ok(json!({
        "iface": q.get("iface").cloned().unwrap_or_else(|| "__all__".into()),
        "interval": INTERVAL,
        "ifaces": all_ifaces,
        "points": points,
        "peak": { "rx": peak_rx, "tx": peak_tx },
    }))
}
