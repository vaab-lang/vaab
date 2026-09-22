//! Helpers for serving static files from route handlers.

use crate::machine::HttpResponse;

pub fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        Some("map") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn read_response(path: &str, status: u16) -> HttpResponse {
    match std::fs::read_to_string(path) {
        Ok(body) => HttpResponse {
            status,
            body,
            content_type: mime_for(path).to_string(),
        },
        Err(_) => HttpResponse {
            status: 404,
            body: "not found".to_string(),
            content_type: "text/plain; charset=utf-8".to_string(),
        },
    }
}
