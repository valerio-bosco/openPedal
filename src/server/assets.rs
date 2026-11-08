//! Interfaccia web incorporata nel binario (cartella `ui/dist`).

use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "ui/dist"]
#[allow_missing = true]
struct Assets;

const MISSING: &str = "<!doctype html><html lang=\"it\"><head><meta charset=\"utf-8\"><title>openPedal</title>\
<style>body{font-family:sans-serif;background:#161616;color:#ddd;max-width:40em;margin:4em auto;padding:0 1em}\
code{background:#000;padding:.2em .5em;border-radius:4px}</style></head><body>\
<h1>openPedal</h1><p>Il server funziona, ma l'interfaccia non è stata compilata.</p>\
<p>Dalla cartella del progetto lancia:</p><p><code>cd ui &amp;&amp; npm install &amp;&amp; npm run build</code></p>\
<p>poi riavvia <code>openpedal ui</code>.</p></body></html>";

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(file) => ([(header::CONTENT_TYPE, mime(path))], file.data.into_owned()).into_response(),
        None if path == "index.html" => Html(MISSING).into_response(),
        None => (StatusCode::NOT_FOUND, "non trovato").into_response(),
    }
}
