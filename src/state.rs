use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: std::sync::Arc<Mutex<Connection>>,
    pub sessions: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, crate::auth::Session>>>,
    pub sys: std::sync::Arc<tokio::sync::Mutex<sysinfo::System>>,
    pub pkg_tx: tokio::sync::broadcast::Sender<String>,
    pub pkg_running: std::sync::Arc<tokio::sync::Mutex<bool>>,
    /// 工作根目录（BIANWRT_WORK_DIR，默认 /etc/bianwrt）
    pub work_dir: PathBuf,
    /// 数据子目录：SQLite、生成的配置文件、插件文件清单
    pub data_dir: PathBuf,
    /// 插件安装目录
    pub plugins_dir: PathBuf,
    /// 插件配置目录（类似 OpenWrt /etc/config）
    pub conf_dir: PathBuf,
}

/// 统一目录布局：<work_dir>/{data,plugins,conf}
pub fn init(work_dir: &PathBuf) -> AppState {
    let data_dir = work_dir.join("data");
    let plugins_dir = work_dir.join("plugins");
    let conf_dir = work_dir.join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&plugins_dir).expect("create plugins dir");
    std::fs::create_dir_all(&conf_dir).expect("create conf dir");
    let db = Connection::open(data_dir.join("bianwrt.db")).expect("open sqlite");
    db.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS fw_zones(name TEXT PRIMARY KEY, ifaces TEXT NOT NULL DEFAULT '[]');
        CREATE TABLE IF NOT EXISTS fw_rules(
            id INTEGER PRIMARY KEY AUTOINCREMENT, zone TEXT NOT NULL,
            proto TEXT NOT NULL DEFAULT 'tcp', port TEXT NOT NULL,
            src TEXT DEFAULT '', action TEXT NOT NULL DEFAULT 'accept',
            comment TEXT DEFAULT '', enabled INTEGER NOT NULL DEFAULT 1);
        CREATE TABLE IF NOT EXISTS fw_forwards(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proto TEXT NOT NULL DEFAULT 'tcp', src_port TEXT NOT NULL,
            dest TEXT NOT NULL, dest_port TEXT DEFAULT '', comment TEXT DEFAULT '',
            enabled INTEGER NOT NULL DEFAULT 1);
        CREATE TABLE IF NOT EXISTS plugins(
            name TEXT PRIMARY KEY, version TEXT NOT NULL, web_mode TEXT DEFAULT 'static',
            has_service INTEGER DEFAULT 0, description TEXT DEFAULT '', author TEXT DEFAULT '',
            homepage TEXT DEFAULT '', installed_at TEXT DEFAULT '');
        CREATE TABLE IF NOT EXISTS dhcp_static(mac TEXT PRIMARY KEY, ip TEXT NOT NULL, hostname TEXT DEFAULT '');
        "#,
    )
    .expect("init schema");

    // seed default zones + settings
    db.execute_batch(
        r#"
        INSERT OR IGNORE INTO fw_zones(name, ifaces) VALUES('wan','[]'), ('lan','[]');
        INSERT OR IGNORE INTO settings(key,value) VALUES('fw_enabled','0');
        INSERT OR IGNORE INTO settings(key,value) VALUES('fw_masquerade','1');
        INSERT OR IGNORE INTO settings(key,value) VALUES('fw_allow_ping','1');
        INSERT OR IGNORE INTO settings(key,value) VALUES('dhcp_config','{}');
        "#,
    )
    .expect("seed");

    let (pkg_tx, _) = tokio::sync::broadcast::channel(256);
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    AppState {
        db: std::sync::Arc::new(Mutex::new(db)),
        sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        sys: std::sync::Arc::new(tokio::sync::Mutex::new(sys)),
        pkg_tx,
        pkg_running: std::sync::Arc::new(tokio::sync::Mutex::new(false)),
        work_dir: work_dir.clone(),
        data_dir,
        plugins_dir,
        conf_dir,
    }
}

pub fn get_setting(db: &Connection, key: &str) -> Option<String> {
    db.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

pub fn set_setting(db: &Connection, key: &str, value: &str) {
    db.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
        [key, value],
    )
    .ok();
}
