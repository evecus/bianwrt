use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::state::AppState;

pub async fn stats(State(state): State<AppState>) -> Response {
    let mut sys = state.sys.lock().await;
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    // small delay for meaningful cpu delta
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sys.refresh_cpu_usage();

    let cpus: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage() as f32).collect();
    let (total, used) = (sys.total_memory(), sys.used_memory());
    let (swap_total, swap_used) = (sys.total_swap(), sys.used_swap());

    // disks via sysinfo
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .unwrap_or(0.0) as u64;

    let disks = sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|d| {
            let m = d.mount_point().to_string_lossy();
            d.total_space() > 0
                && !m.starts_with("/proc")
                && !m.starts_with("/sys")
                && !m.starts_with("/dev")
                && !m.starts_with("/run")
                && m != "/boot/efi"
        })
        // collapse bind mounts of the same underlying filesystem
        .map(|d| {
            json!({
                "mount": d.mount_point().display().to_string(),
                "fs": d.file_system().to_string_lossy(),
                "total": d.total_space(),
                "free": d.available_space(),
            })
        })
        .collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    let disks: Vec<serde_json::Value> = disks
        .into_iter()
        .filter(|v| {
            let k = format!("{}-{}-{}", v["fs"].as_str().unwrap_or(""), v["total"], v["free"]);
            seen.insert(k)
        })
        .collect();

    // network totals
    let nets = sysinfo::Networks::new_with_refreshed_list();
    let net: Vec<serde_json::Value> = nets
        .iter()
        .map(|(name, d)| {
            json!({
                "name": name,
                "rx_total": d.total_received(),
                "tx_total": d.total_transmitted(),
                "rx_speed": d.received(),
                "tx_speed": d.transmitted(),
            })
        })
        .collect();

    Json(json!({
        "hostname": sysinfo::System::host_name().unwrap_or_default(),
        "os": sysinfo::System::long_os_version().unwrap_or_default(),
        "kernel": sysinfo::System::kernel_version().unwrap_or_default(),
        "uptime": uptime,
        "load": {
            "1": sysinfo::System::load_average().one,
            "5": sysinfo::System::load_average().five,
            "15": sysinfo::System::load_average().fifteen,
        },
        "cpu": {
            "cores": cpus.len(),
            "usage": cpus.first().copied().unwrap_or(0.0),
            "per_core": cpus,
            "brand": sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default(),
        },
        "mem": { "total": total, "used": used, "swap_total": swap_total, "swap_used": swap_used },
        "disks": disks,
        "net": net,
    }))
    .into_response()
}
