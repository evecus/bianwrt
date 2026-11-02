//! 前端静态资源：编译期内嵌进二进制（rust-embed）。
//! web/dist 在 cargo build 时必须已构建（CI 中先 pnpm build 再 cargo build）。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub(crate) fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "map" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn file_response(rel: &str, data: &[u8]) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, mime_of(rel)),
            (axum::http::header::CACHE_CONTROL, cache_control(rel)),
        ],
        data.to_vec(),
    )
        .into_response()
}

fn cache_control(rel: &str) -> &'static str {
    // vite 产物带内容哈希，可长缓存；index.html 不缓存保证发版即生效
    if rel == "index.html" {
        "no-cache"
    } else if rel.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    }
}

/// 站点 fallback handler：/ 及所有非 /api、/plugins 路径
pub async fn serve(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // 精确命中
    if let Some(file) = Assets::get(path) {
        return file_response(path, &file.data);
    }
    // 目录形式 /xxx -> /xxx/index.html
    if let Some(file) = Assets::get(&format!("{}/index.html", path)) {
        return file_response(&format!("{}/index.html", path), &file.data);
    }
    // SPA 回落：未匹配路由交给前端路由器
    if let Some(file) = Assets::get("index.html") {
        return file_response("index.html", &file.data);
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "前端资源未内嵌：请先在 web/ 目录执行 pnpm build 后重新编译",
    )
        .into_response()
}
