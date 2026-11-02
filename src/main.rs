mod auth;
mod dhcpdns;
mod firewall;
mod netif;
mod pkg;
mod plugins;
mod state;
mod system;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use state::AppState;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

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

    let home = PathBuf::from(std::env::var("BIANWRT_HOME").unwrap_or_else(|_| "/var/lib/bianwrt".into()));
    let plugins_dir = PathBuf::from(
        std::env::var("BIANWRT_PLUGIN_DIR").unwrap_or_else(|_| "/opt/bianwrt/plugins".into()),
    );
    // 插件配置目录（类似 OpenWrt 的 /etc/config，卸载默认保留）
    let etc_dir = PathBuf::from(
        std::env::var("BIANWRT_PLUGIN_ETC").unwrap_or_else(|_| "/etc/bianwrt/plugins".into()),
    );
    let web_dist = std::env::var("BIANWRT_WEB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
            let cands = vec![
                exe.clone().map(|p| p.join("web/dist")),
                Some(PathBuf::from("./web/dist")),
            ];
            cands
                .into_iter()
                .flatten()
                .find(|p| p.join("index.html").exists())
                .unwrap_or_else(|| PathBuf::from("./web/dist"))
        });

    let st = state::init(&home, plugins_dir, etc_dir, web_dist.clone());
    let port: u16 = std::env::var("BIANWRT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8088);

    tracing::info!("bianwrt 启动: home={} plugins={} web={}", home.display(), st.plugins_dir.display(), web_dist.display());

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
        .layer(middleware::from_fn_with_state(
            st.clone(),
            auth::auth_mw,
        ));

    let plugin_pages = Router::new()
        .route("/plugins/{name}", get(plugins::plugin_static))
        .route("/plugins/{name}/", get(plugins::plugin_static))
        .route("/plugins/{name}/{*rest}", get(plugins::plugin_static_rest));

    let web = ServeDir::new(&web_dist)
        .not_found_service(ServeFile::new(web_dist.join("index.html")));

    let app = Router::new()
        .nest("/api", api)
        .merge(plugin_pages)
        .fallback_service(web)
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
        let path = st.home.join("bianwrt.nft");
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
