//! 侧边栏菜单配置：标题 / 图标 / 显隐 / 排序可自定义。
//!
//! 默认菜单与可选图标来自项目根目录 menu-defaults.json（前后端共用同一份）。
//! 用户配置以 JSON 存入 settings.menu_config，随备份 / 恢复一起保存。

use axum::{extract::State, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

use crate::cmd;
use crate::state::{get_setting, set_setting, AppState};

const DEFAULTS_RAW: &str = include_str!("../menu-defaults.json");

#[derive(Serialize, Deserialize, Clone)]
pub struct MenuItem {
    pub key: String,
    pub label: String,
    pub icon: String,
    pub visible: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MenuGroup {
    pub key: String,
    pub label: String,
    pub icon: String,
    pub visible: bool,
    pub items: Vec<MenuItem>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MenuConfig {
    pub brand: String,
    pub groups: Vec<MenuGroup>,
}

#[derive(Deserialize)]
struct DefaultsFile {
    brand: String,
    icons: Vec<String>,
    groups: Vec<MenuGroup>,
}

fn defaults() -> (MenuConfig, Vec<String>) {
    let f: DefaultsFile = serde_json::from_str(DEFAULTS_RAW).expect("menu-defaults.json 合法");
    (
        MenuConfig {
            brand: f.brand,
            groups: f.groups,
        },
        f.icons,
    )
}

fn known_keys(cfg: &MenuConfig) -> HashSet<String> {
    cfg.groups
        .iter()
        .flat_map(|g| g.items.iter().map(|i| i.key.clone()))
        .collect()
}

/// 已安装且带页面的插件对应的菜单 key，允许出现在自定义配置里。
fn plugin_keys(state: &AppState) -> Vec<String> {
    let db = state.db.lock().unwrap();
    let mut stmt = match db.prepare("SELECT name, web_mode FROM plugins") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .ok()
    .map(|rows| {
        rows.flatten()
            .filter(|(_, mode)| mode != "none")
            .map(|(name, _)| format!("/plugin-page/{}", name))
            .collect()
    })
    .unwrap_or_default()
}

pub async fn get_config(State(state): State<AppState>) -> Response {
    let (def, icons) = defaults();
    // 先取插件 key，避免持 db 锁时再次加锁造成死锁
    let pkeys = plugin_keys(&state);
    let config: MenuConfig = {
        let db = state.db.lock().unwrap();
        get_setting(&db, "menu_config")
            .and_then(|s| serde_json::from_str(&s).ok())
            .and_then(|v| validate(&v, &def, &icons, &pkeys).ok().map(|_| v))
            .unwrap_or_else(|| def.clone())
    };
    cmd::ok(json!({
        "config": config,
        "defaults": def,
        "icons": icons,
        "plugin_keys": pkeys,
    }))
}

pub async fn put_config(
    State(state): State<AppState>,
    Json(cfg): Json<MenuConfig>,
) -> Response {
    let (def, icons) = defaults();
    if let Err(e) = validate(&cfg, &def, &icons, &plugin_keys(&state)) {
        return cmd::err(e);
    }
    let text = serde_json::to_string(&cfg).unwrap_or_default();
    let db = state.db.lock().unwrap();
    set_setting(&db, "menu_config", &text);
    cmd::ok(json!({ "ok": true }))
}

pub async fn reset(State(state): State<AppState>) -> Response {
    let db = state.db.lock().unwrap();
    db.execute("DELETE FROM settings WHERE key='menu_config'", [])
        .ok();
    cmd::ok(json!({ "ok": true }))
}

fn label_ok(s: &str) -> bool {
    !s.is_empty() && cmd::safe_text(s, 32)
}

/// 严格校验：只允许调整已知页面的标题/图标/显隐/顺序，不允许注入任意路由。
fn validate(
    cfg: &MenuConfig,
    def: &MenuConfig,
    icons: &[String],
    plugins: &[String],
) -> Result<(), String> {
    if !label_ok(&cfg.brand) {
        return Err("品牌标题须为 1~32 个有效字符".into());
    }
    if cfg.groups.len() != def.groups.len() || cfg.groups.is_empty() {
        return Err("分组数量必须与默认一致".into());
    }
    let def_group_keys: HashSet<&str> = def.groups.iter().map(|g| g.key.as_str()).collect();
    let allowed_item_keys: HashSet<String> = known_keys(def)
        .into_iter()
        .chain(plugins.iter().cloned())
        .collect();
    let icon_set: HashSet<&str> = icons.iter().map(|s| s.as_str()).collect();
    let mut seen_groups = HashSet::new();
    let mut seen_items = HashSet::new();
    for g in &cfg.groups {
        if !def_group_keys.contains(g.key.as_str()) || !seen_groups.insert(g.key.clone()) {
            return Err(format!("存在未知或重复的分组: {}", g.key));
        }
        if !label_ok(&g.label) {
            return Err(format!("分组 {} 标题非法", g.key));
        }
        if !icon_set.contains(g.icon.as_str()) {
            return Err(format!("分组 {} 图标不在可选列表内", g.key));
        }
        if g.items.len() > 200 {
            return Err("单个分组菜单项过多".into());
        }
        for i in &g.items {
            if !allowed_item_keys.contains(&i.key) || !seen_items.insert(i.key.clone()) {
                return Err(format!("存在未知或重复的菜单项: {}", i.key));
            }
            if !label_ok(&i.label) {
                return Err(format!("菜单项 {} 标题非法", i.key));
            }
            if !icon_set.contains(i.icon.as_str()) {
                return Err(format!("菜单项 {} 图标不在可选列表内", i.key));
            }
        }
    }
    // 所有默认内置页面都必须保留（可隐藏但不可丢弃），避免误删导致页面无法进入
    for k in known_keys(def) {
        if !seen_items.contains(&k) {
            return Err(format!("缺少内置菜单项: {}（隐藏请用 visible=false）", k));
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn as_value(cfg: &MenuConfig) -> Value {
    serde_json::to_value(cfg).unwrap_or(json!({}))
}
