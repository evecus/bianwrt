mod auth;
mod backup;
mod clients;
mod cmd;
mod cron;
mod dhcpdns;
mod diag;
mod dnsfilter;
mod firewall;
mod netcfg;
mod netif;
mod pkg;
mod plugins;
mod process;
mod services;
mod state;
mod sysadmin;
mod syslog;
mod system;
mod webassets;
mod wireguard;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use state::AppState;
use std::path::PathBuf;

fn main() {
    let n = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(n)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main());
}

async fn async_main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 统一工作目录：<BIANWRT_WORK_DIR>/{data,plugins,conf}
    let work_dir = PathBuf::from(
        std::env::var("BIANWRT_WORK_DIR").unwrap_or_else(|_| "/etc/bianwrt".into()),
    );
    let st = state::init(&work_dir);
    let port: u16 = std::env::var("BIANWRT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8088);

    tracing::info!("bianwrt 启动: work_dir={} (data/plugins/conf)", st.work_dir.display());

    // boot-time apply of persisted firewall / dnsmasq config
    tokio::spawn(boot_apply(st.clone()));

    let api = Router::new()
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/status", get(auth::status))
        .route("/system/stats", get(system::stats))
        .route("/network/interfaces", get(netif::interfaces))
        .route("/network/routes", get(netif::routes))
        .route("/firewall", get(firewall::get).put(firewall::put))
        .route("/firewall/apply", post(firewall::apply))
        .route("/dhcp", get(dhcpdns::get).put(dhcpdns::put))
        .route("/dhcp/apply", post(dhcpdns::apply))
        .route("/dhcp/static", post(dhcpdns::add_static))
        .route("/dhcp/static/{mac}", delete(dhcpdns::del_static))
        .route("/dhcp/leases", get(dhcpdns::leases))
        .route("/pkg/installed", get(pkg::installed))
        .route("/pkg/search", get(pkg::search))
        .route("/pkg/action", post(pkg::action))
        .route("/pkg/ws", get(pkg::ws))
        .route("/plugins", get(plugins::list))
        .route("/plugins/install", post(plugins::install))
        .route("/plugins/{name}", delete(plugins::uninstall))
        .route("/plugins/{name}/{op}", post(plugins::action))
        // ── 状态：日志 / 进程 / 在线客户端 ──────────────────────────
        .route("/logs", get(syslog::read))
        .route("/logs/follow", get(syslog::follow))
        .route("/processes", get(process::list))
        .route("/processes/kill", post(process::kill))
        .route("/clients", get(clients::list))
        .route("/clients/connections", get(clients::connections))
        .route("/clients/wol", post(clients::wol))
        // ── 系统：服务 / 计划任务 / 备份 / 系统设置 ─────────────────
        .route("/services", get(services::list))
        .route("/services/action", post(services::action))
        .route("/services/status", get(services::status))
        .route("/cron", get(cron::get).put(cron::put))
        .route("/backup", get(backup::list).post(backup::create))
        .route("/backup/file/{name}", get(backup::download).delete(backup::delete))
        .route("/backup/upload", post(backup::upload))
        .route("/backup/restore", post(backup::restore))
        .route("/system/info", get(sysadmin::info))
        .route("/system/hostname", post(sysadmin::hostname))
        .route("/system/timezone", post(sysadmin::timezone))
        .route("/system/timezones", get(sysadmin::timezones))
        .route("/system/ntp", post(sysadmin::ntp))
        .route("/system/password", post(sysadmin::password))
        .route("/system/sshkeys", put(sysadmin::ssh_keys_put))
        .route("/system/power", post(sysadmin::power))
        // ── 网络：诊断 / 路由与接口 / WireGuard / DNS 过滤 ───────────
        .route("/diag/tools", get(diag::tools))
        .route("/diag/ping", post(diag::ping))
        .route("/diag/traceroute", post(diag::traceroute))
        .route("/diag/nslookup", post(diag::nslookup))
        .route("/diag/tcp", post(diag::tcp))
        .route("/diag/http", post(diag::http))
        .route(
            "/netcfg/routes",
            get(netcfg::routes)
                .post(netcfg::route_add)
                .delete(netcfg::route_del),
        )
        .route("/netcfg/interfaces", get(netcfg::interfaces))
        .route("/netcfg/interfaces/link", post(netcfg::link))
        .route("/netcfg/interfaces/addr", post(netcfg::addr))
        .route(
            "/netcfg/persist",
            get(netcfg::persist_list).put(netcfg::persist_put),
        )
        .route("/netcfg/persist/generate", post(netcfg::generate))
        .route("/netcfg/persist/apply", post(netcfg::persist_apply))
        .route("/netcfg/persist/file/{name}", delete(netcfg::persist_del))
        .route("/wg/status", get(wireguard::status))
        .route("/wg/interfaces", get(wireguard::interfaces))
        .route("/wg/interface", put(wireguard::put))
        .route("/wg/interface/{name}", delete(wireguard::del))
        .route("/wg/action", post(wireguard::action))
        .route("/wg/keygen", post(wireguard::keygen))
        .route("/dnsfilter", get(dnsfilter::get).put(dnsfilter::put))
        .route("/dnsfilter/apply", post(dnsfilter::apply))
        .layer(middleware::from_fn_with_state(
            st.clone(),
            auth::auth_mw,
        ));

    let plugin_pages = Router::new()
        .route("/plugins/{name}", get(plugins::plugin_static))
        .route("/plugins/{name}/", get(plugins::plugin_static))
        .route("/plugins/{name}/{*rest}", get(plugins::plugin_static_rest));

    let app = Router::new()
        .nest("/api", api)
        .merge(plugin_pages)
        .fallback(webassets::serve)
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(st.clone());

    tokio::spawn(auth::session_gc(st));

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind port");
    tracing::info!("listening on http://0.0.0.0:{}", port);
    axum::serve(listener, app).await.unwrap();
}

async fn boot_apply(st: AppState) {
    // firewall
    let cfg = firewall::load_config(&st.db.lock().unwrap());
    if cfg.enabled {
        let script = firewall::render_nft(&cfg);
        let path = st.data_dir.join("bianwrt.nft");
        let _ = std::fs::write(&path, &script);
        let out = tokio::process::Command::new("nft")
            .args(["-f"])
            .arg(&path)
            .output()
            .await;
        tracing::info!("启动时应用防火墙: {:?}", out.map(|o| o.status.success()));
    }
    // dnsmasq (only reload config we manage)
    let cfg: dhcpdns::DhcpConfig = {
        let db = st.db.lock().unwrap();
        state::get_setting(&db, "dhcp_config")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    };
    if cfg.enabled {
        let mut statics = Vec::new();
        {
            let db = st.db.lock().unwrap();
            let mut stmt = db.prepare("SELECT mac,ip,hostname FROM dhcp_static").unwrap();
            for r in stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .unwrap()
                .flatten()
            {
                statics.push(r);
            }
        }
        let content = dhcpdns::render_dnsmasq(&cfg, &statics);
        if std::path::Path::new("/etc/dnsmasq.d").is_dir() {
            let _ = std::fs::write("/etc/dnsmasq.d/bianwrt.conf", content);
        }
    }
}
