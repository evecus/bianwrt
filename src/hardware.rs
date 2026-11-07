//! Status → Hardware：温度/风扇/电压传感器、CPU 频率、LED 控制（sysfs）。

use axum::{extract::State, response::Response, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{cmd, state::AppState};

fn read_trim(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn millideg_to_c(v: i64) -> f64 {
    // 内核统一以毫摄氏度上报；个别平台已是摄氏度，用 >1000 判断
    if v.abs() > 1000 {
        v as f64 / 1000.0
    } else {
        v as f64
    }
}

/// 扫描 thermal_zone 与 hwmon 的 temp/fan/in 传感器。
pub fn scan_temps() -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/class/thermal") {
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let name = p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let raw = read_trim(&p.join("temp"));
            let Ok(v) = raw.parse::<i64>() else { continue };
            let label = read_trim(&p.join("type"));
            items.push(json!({
                "source": "thermal",
                "name": name,
                "label": if label.is_empty() { name.clone() } else { label },
                "kind": "temp",
                "celsius": millideg_to_c(v),
                "raw": v,
                "unit": "°C",
            }));
        }
    }
    if let Ok(rd) = std::fs::read_dir("/sys/class/hwmon") {
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let chip = read_trim(&p.join("name"));
            let Ok(files) = std::fs::read_dir(&p) else { continue };
            let mut names: Vec<String> = files.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            for n in names {
                let add = |items: &mut Vec<Value>, kind: &str, idx: &str, raw: String, unit: &str| {
                    let Ok(v) = raw.parse::<i64>() else { return };
                    let label_path = p.join(format!("{}{}_label", kind, idx));
                    let label = read_trim(&label_path);
                    let value = if kind == "temp" { millideg_to_c(v) } else { v as f64 };
                    items.push(json!({
                        "source": "hwmon",
                        "name": format!("{}/{}", chip, n),
                        "label": if label.is_empty() { format!("{} {} {}", chip, kind, idx) } else { label },
                        "kind": kind,
                        "value": value,
                        "raw": v,
                        "unit": unit,
                    }));
                };
                if let Some(idx) = n.strip_prefix("temp").and_then(|s| s.strip_suffix("_input")) {
                    add(&mut items, "temp", idx, read_trim(&p.join(&n)), "°C");
                } else if let Some(idx) = n.strip_prefix("fan").and_then(|s| s.strip_suffix("_input")) {
                    add(&mut items, "fan", idx, read_trim(&p.join(&n)), "RPM");
                } else if let Some(idx) = n.strip_prefix("in").and_then(|s| s.strip_suffix("_input")) {
                    add(&mut items, "in", idx, read_trim(&p.join(&n)), "mV");
                }
            }
        }
    }
    items
}

fn led_dir(name: &str) -> Option<std::path::PathBuf> {
    if !cmd::safe(name, 96) {
        return None;
    }
    let p = std::path::Path::new("/sys/class/leds").join(name);
    if p.is_dir() { Some(p) } else { None }
}

/// 扫描 /sys/class/leds，读取亮度、最大亮度与可用 trigger。
pub fn scan_leds() -> Vec<Value> {
    let mut items = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/leds") else { return items };
    let mut dirs: Vec<_> = rd.flatten().collect();
    dirs.sort_by_key(|e| e.file_name());
    for e in dirs {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let brightness = read_trim(&p.join("brightness")).parse::<u32>().unwrap_or(0);
        let max = read_trim(&p.join("max_brightness")).parse::<u32>().unwrap_or(1);
        let trig_raw = read_trim(&p.join("trigger"));
        let triggers: Vec<String> = trig_raw
            .split_whitespace()
            .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let current = trig_raw
            .split_whitespace()
            .find(|s| s.starts_with('['))
            .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
            .unwrap_or_default();
        items.push(json!({
            "name": name,
            "brightness": brightness,
            "max_brightness": max,
            "trigger": current,
            "triggers": triggers,
            "writable": std::fs::OpenOptions::new().write(true).open(p.join("brightness")).is_ok(),
        }));
    }
    items
}

fn cpu_info() -> Value {
    let mut freqs = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/devices/system/cpu") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with("cpu") || n.len() <= 3 || !n[3..].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let khz = read_trim(&e.path().join("cpufreq/scaling_cur_freq"))
                .parse::<u64>()
                .unwrap_or(0);
            if khz > 0 {
                freqs.push(khz);
            }
        }
    }
    let avg = if freqs.is_empty() { 0.0 } else { freqs.iter().sum::<u64>() as f64 / freqs.len() as f64 / 1000.0 };
    let load = read_trim(std::path::Path::new("/proc/loadavg"));
    let uptime = read_trim(std::path::Path::new("/proc/uptime"))
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    json!({
        "cores_with_freq": freqs.len(),
        "avg_mhz": (avg * 10.0).round() / 10.0,
        "loadavg": load,
        "uptime_secs": uptime as u64,
    })
}

pub async fn temps() -> Response {
    let items = scan_temps();
    let temps: Vec<f64> = items.iter()
        .filter(|x| x["kind"] == "temp")
        .filter_map(|x| {
            x["celsius"].as_f64().or_else(|| x["value"].as_f64())
        })
        .collect();
    let max = temps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    cmd::ok(json!({
        "items": items,
        "max": if max.is_finite() { max } else { 0.0 },
        "count": items.len(),
        "msg": if items.is_empty() { "未发现硬件传感器（虚拟机/容器通常无 /sys/class/hwmon 暴露）" } else { "" },
    }))
}

pub async fn leds() -> Response {
    let items = scan_leds();
    cmd::ok(json!({
        "items": items,
        "count": items.len(),
        "msg": if items.is_empty() { "未发现 LED 设备（/sys/class/leds 为空，需真实硬件与 root 权限）" } else { "" },
    }))
}

#[derive(Deserialize)]
pub struct LedReq {
    pub name: String,
    pub brightness: Option<u32>,
    pub trigger: Option<String>,
}

pub async fn led_set(Json(req): Json<LedReq>) -> Response {
    let Some(dir) = led_dir(&req.name) else {
        return cmd::err("LED 名称非法或设备不存在");
    };
    let mut changed = Vec::new();
    if let Some(b) = req.brightness {
        let max = read_trim(&dir.join("max_brightness")).parse::<u32>().unwrap_or(1);
        if b > max {
            return cmd::err(format!("亮度须在 0-{}", max));
        }
        match std::fs::write(dir.join("brightness"), b.to_string()) {
            Ok(_) => changed.push(format!("brightness={}", b)),
            Err(e) => return cmd::fail(format!("写入 brightness 失败（需要 root）：{}", e)),
        }
    }
    if let Some(t) = req.trigger {
        let raw = read_trim(&dir.join("trigger"));
        let allowed: Vec<String> = raw
            .split_whitespace()
            .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
            .collect();
        if !allowed.contains(&t) {
            return cmd::err("trigger 不在内核提供的可选值中");
        }
        match std::fs::write(dir.join("trigger"), &t) {
            Ok(_) => changed.push(format!("trigger={}", t)),
            Err(e) => return cmd::fail(format!("写入 trigger 失败（需要 root）：{}", e)),
        }
    }
    if changed.is_empty() {
        return cmd::err("未指定 brightness 或 trigger");
    }
    cmd::ok(json!({ "ok": true, "changed": changed }))
}

/// 仪表盘用聚合信息（不需要 State，但保留签名一致性）。
pub async fn summary(State(_state): State<AppState>) -> Response {
    let sensors = scan_temps();
    let temps: Vec<f64> = sensors.iter()
        .filter(|x| x["kind"] == "temp")
        .filter_map(|x| x["celsius"].as_f64().or_else(|| x["value"].as_f64()))
        .collect();
    let fans: Vec<f64> = sensors.iter()
        .filter(|x| x["kind"] == "fan")
        .filter_map(|x| x["value"].as_f64())
        .collect();
    let max = temps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let leds = scan_leds();
    cmd::ok(json!({
        "temp_max": if max.is_finite() { max } else { 0.0 },
        "temp_count": temps.len(),
        "fan_count": fans.len(),
        "fan_max": fans.iter().copied().fold(0.0, f64::max),
        "led_count": leds.len(),
        "sensor_count": sensors.len(),
        "cpu": cpu_info(),
    }))
}
