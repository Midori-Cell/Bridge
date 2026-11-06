//! The phone-facing HTTPS server: TLS accept loop plus the REST routes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::error::ApiError;
use crate::model::*;
use crate::state::{NoteError, PairError, Shared};
use crate::transfer::{parse_range, RangeSpec};

const MAX_JSON_BYTES: usize = 512 * 1024;
const MAX_WAIT_SECS: f64 = 30.0;
const MAX_CONNECTIONS: usize = 256;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const PROTOCOL_VERSION: u32 = 1;

const PHONE_HTML: &str = include_str!("../web/phone.html");
const PHONE_JS: &str = include_str!("../web/phone.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

const CSP_PAGE: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
                        frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
const CSP_DOWNLOAD: &str = "default-src 'none'; sandbox";

// ------------------------------------------------------------------ accept loop

/// Serve `app` on `listener`. The same loop runs the phone port and the PC port; on the PC port the
/// TLS layer has already demanded a client certificate, whose fingerprint is attached to every request.
pub(crate) async fn serve(
    app: Router,
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    cancel: CancellationToken,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, remote) = tokio::select! {
            res = listener.accept() => match res {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            _ = cancel.cancelled() => break,
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue; // too many connections: drop this one
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = stream.set_nodelay(true);
            // Handshake in this task with a deadline, so a stalled client blocks nobody else.
            let Ok(Ok(tls_stream)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await else {
                return;
            };
            let fp = crate::identity::peer_fingerprint(tls_stream.get_ref().1);
            let app = app
                .layer(axum::Extension(crate::peer::PeerFp(fp)))
                .layer(axum::Extension(crate::peer::PeerIp(remote.ip())));
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(tls_stream), TowerToHyperService::new(app));
            tokio::pin!(conn);
            tokio::select! {
                res = conn.as_mut() => {
                    if let Err(e) = res {
                        tracing::debug!("connection ended: {e}");
                    }
                }
                _ = cancel.cancelled() => {}
            }
        });
    }
}

// ------------------------------------------------------------------ extractors

pub(crate) struct Auth(pub Device);

impl FromRequestParts<Arc<Shared>> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<Shared>) -> Result<Self, ApiError> {
        let header = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        let token = match header.split_once(' ') {
            Some((scheme, token)) if scheme.eq_ignore_ascii_case("bearer") => token.trim(),
            _ => "",
        };
        state.auth(token).map(Auth).ok_or_else(ApiError::unauthorized)
    }
}

/// A JSON object body. Content type, size and shape are checked here, once.
pub(crate) struct JsonBody(pub Map<String, Value>);

impl FromRequest<Arc<Shared>> for JsonBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, _state: &Arc<Shared>) -> Result<Self, ApiError> {
        let ctype = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
        if ctype.split(';').next().unwrap_or("").trim() != "application/json" {
            return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "json_required"));
        }
        let bytes = axum::body::to_bytes(req.into_body(), MAX_JSON_BYTES)
            .await
            .map_err(|_| ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large"))?;
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => Ok(JsonBody(map)),
            _ => Err(ApiError::bad_request("bad_json")),
        }
    }
}

// ------------------------------------------------------------------ router

pub(crate) fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/", get(|| async { asset(PHONE_HTML, "text/html; charset=utf-8") }))
        .route("/phone.js", get(|| async { asset(PHONE_JS, "application/javascript; charset=utf-8") }))
        .route("/style.css", get(|| async { asset(STYLE_CSS, "text/css; charset=utf-8") }))
        .route("/api/pair", post(pair))
        .route("/api/ping", get(ping))
        .route("/api/unpair", post(unpair))
        .route("/api/notes", get(notes_get).post(notes_post))
        .route("/api/notes/{mid}", axum::routing::delete(notes_delete))
        .route("/api/files", post(files_create))
        .route("/api/files/{id}", get(files_get).put(files_put).delete(files_delete))
        .route("/api/files/{id}/content", get(files_content))
        .route("/api/files/{id}/ticket", post(files_ticket))
        .route("/dl/{ticket}/{id}", get(download))
        .fallback(|| async { ApiError::not_found() })
        .layer(middleware::map_response(security_headers))
        .with_state(shared)
}

fn asset(body: &'static str, ctype: &'static str) -> Response {
    ([(header::CONTENT_TYPE, ctype)], body).into_response()
}

pub(crate) async fn security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    let defaults: [(HeaderName, &'static str); 5] = [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::CONTENT_SECURITY_POLICY, CSP_PAGE),
    ];
    for (name, value) in defaults {
        h.entry(name).or_insert(HeaderValue::from_static(value));
    }
    res
}

// ------------------------------------------------------------------ pairing and notes

async fn pair(State(s): State<Arc<Shared>>, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let code = b.get("code").and_then(Value::as_str).unwrap_or("");
    let name = b.get("name").and_then(Value::as_str).unwrap_or("");
    match s.pair(code, name) {
        Ok((id, token)) => Ok(Json(json!({ "token": token, "device_id": id, "fingerprint": s.fingerprint, "pc": s.pc_name() }))),
        Err(PairError::Locked) => Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "locked")),
        Err(PairError::Invalid) => Err(ApiError::new(StatusCode::FORBIDDEN, "invalid")),
        Err(PairError::TooManyDevices) => Err(ApiError::new(StatusCode::FORBIDDEN, "too_many_devices")),
    }
}

async fn ping(State(s): State<Arc<Shared>>, Auth(dev): Auth) -> Json<Value> {
    Json(json!({ "ok": true, "device": dev.name, "pc": s.pc_name(), "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL_VERSION }))
}

/// The phone unpairs itself: this PC forgets it right away.
async fn unpair(State(s): State<Arc<Shared>>, Auth(dev): Auth) -> Json<Value> {
    Json(json!({ "unpaired": s.revoke(&dev.id) }))
}

async fn notes_get(
    State(s): State<Arc<Shared>>,
    Auth(dev): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let since = match q.get("since") {
        None => None,
        Some(v) => Some(v.parse::<u64>().map_err(|_| ApiError::bad_request("bad_query"))?),
    };
    let wait = match q.get("wait") {
        None => 0.0,
        Some(v) => v.parse::<f64>().ok().filter(|w| w.is_finite()).ok_or_else(|| ApiError::bad_request("bad_query"))?,
    }
    .clamp(0.0, MAX_WAIT_SECS);
    // `del`: how many deletions the phone has already seen. Without it, no deletions are reported.
    let del_seen = match q.get("del") {
        None => None,
        Some(v) => Some(v.parse::<u64>().map_err(|_| ApiError::bad_request("bad_query"))?),
    };

    let deadline = Instant::now() + Duration::from_secs_f64(wait);
    let mut wake = s.wake_receiver();
    loop {
        wake.borrow_and_update(); // mark "seen" before reading, so no change can slip between read and wait
        if !s.device_exists(&dev.id) {
            return Err(ApiError::unauthorized()); // unpaired while waiting
        }
        // the phone asks for what comes after `since`, so it has stored everything up to there
        if let Some(sn) = since {
            s.ack_notes(&dev.id, sn);
        }
        let notes: Vec<Note> = s.notes_since(&dev.id, since, 100).iter().map(Note::for_phone).collect();
        let (deleted, del) = s.deleted_since(&dev.id, del_seen.unwrap_or(u64::MAX));
        let remaining = deadline.saturating_duration_since(Instant::now());
        if since.is_none() || !notes.is_empty() || !deleted.is_empty() || remaining.is_zero() {
            let last = notes.last().map(|n| n.id).unwrap_or(since.unwrap_or(0));
            return Ok(Json(json!({ "notes": notes, "last_id": last, "deleted": deleted, "del": del })));
        }
        if !matches!(tokio::time::timeout(remaining, wake.changed()).await, Ok(Ok(()))) {
            // timed out (or the server is shutting down): answer with what we have
            let last = since.unwrap_or(0);
            return Ok(Json(json!({ "notes": Vec::<Note>::new(), "last_id": last, "deleted": Vec::<String>::new(), "del": del })));
        }
    }
}

async fn notes_post(State(s): State<Arc<Shared>>, Auth(dev): Auth, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let text = b.get("text").and_then(Value::as_str).ok_or_else(|| ApiError::bad_request("empty_text"))?;
    let reply_to = b.get("reply_to").and_then(Value::as_str);
    match s.add_text_note(&dev.id, &dev.name, text, reply_to) {
        Ok(note) => Ok(Json(json!({ "note": note.for_phone() }))),
        Err(NoteError::Empty) => Err(ApiError::bad_request("empty_text")),
        Err(NoteError::TooLong) => Err(ApiError::bad_request("text_too_long")),
    }
}

/// The phone deletes one of its messages: it goes from the PC's view too.
async fn notes_delete(State(s): State<Arc<Shared>>, Auth(dev): Auth, Path(mid): Path<String>) -> Result<Json<Value>, ApiError> {
    if s.delete_note(&dev.id, &mid) { Ok(Json(json!({ "deleted": true }))) } else { Err(ApiError::not_found()) }
}

// ------------------------------------------------------------------ files: phone -> PC

async fn files_create(State(s): State<Arc<Shared>>, Auth(dev): Auth, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let name = b.get("name").and_then(Value::as_str).ok_or_else(|| ApiError::bad_request("bad_name"))?;
    let size = b.get("size").and_then(Value::as_u64).ok_or_else(|| ApiError::bad_request("bad_size"))?;
    let sha = match b.get("sha256") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.clone()),
        Some(_) => return Err(ApiError::bad_request("bad_sha256")),
    };
    let out = s.create_incoming(&dev, name, size, sha).await?;
    let file = out.file.map(|mut f| {
        f.local_path = None;
        f
    });
    Ok(Json(json!({ "id": out.id, "name": out.name, "offset": 0, "done": out.done, "state": out.state, "file": file })))
}

async fn files_put(
    State(s): State<Arc<Shared>>,
    Auth(dev): Auth,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let offset = match q.get("offset") {
        None => 0,
        Some(v) => v.parse::<u64>().map_err(|_| ApiError::bad_request("bad_query"))?,
    };
    let content_length = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let out = s.put_incoming(&dev, &id, offset, content_length, body).await?;
    let file = out.file.map(|mut f| {
        f.local_path = None;
        f
    });
    Ok(Json(json!({ "received": out.received, "done": out.done, "file": file })))
}

async fn files_get(State(s): State<Arc<Shared>>, Auth(dev): Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if let Some(v) = s.incoming_status(&dev.id, &id) {
        return Ok(Json(v));
    }
    let rec = s.offer_for(&dev.id, &id).ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "id": rec.id, "name": rec.name, "size": rec.size, "sha256": rec.sha256, "dir": "to_phone", "state": "ready" })))
}

async fn files_delete(State(s): State<Arc<Shared>>, Auth(dev): Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    s.cancel_incoming(&dev.id, &id)?;
    Ok(Json(json!({ "cancelled": true })))
}

// ------------------------------------------------------------------ files: PC -> phone

async fn files_ticket(State(s): State<Arc<Shared>>, Auth(dev): Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    s.offer_for(&dev.id, &id).ok_or_else(ApiError::not_found)?;
    let ticket = s.new_ticket(&dev.id, &id);
    Ok(Json(json!({ "ticket": ticket, "url": format!("/dl/{ticket}/{id}"), "expires_in": 300 })))
}

async fn files_content(
    State(s): State<Arc<Shared>>,
    Auth(dev): Auth,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve_offer(&s, &dev, &id, &headers).await
}

async fn download(
    State(s): State<Arc<Shared>>,
    Path((ticket, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let dev = s.check_ticket(&ticket, &id).ok_or_else(ApiError::unauthorized)?;
    serve_offer(&s, &dev, &id, &headers).await
}

const ATTR_CHARS: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn content_disposition(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' { c } else { '_' }).collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", utf8_percent_encode(name, ATTR_CHARS))
}

async fn serve_offer(s: &Arc<Shared>, dev: &Device, id: &str, headers: &HeaderMap) -> Result<Response, ApiError> {
    let (rec, file) = s.open_offer(&dev.id, id).await?;
    let size = rec.size;
    let etag = format!("\"{}\"", rec.sha256);

    // If-Range: only honour the Range header if the client's copy is the same file.
    let range_ok = headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).is_none_or(|v| v == etag);
    let spec = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(r) if range_ok => parse_range(r, size),
        _ => RangeSpec::Full,
    };

    let (status, start, end) = match spec {
        RangeSpec::Full => (StatusCode::OK, 0, size.saturating_sub(1)),
        RangeSpec::Partial(a, b) => (StatusCode::PARTIAL_CONTENT, a, b),
        RangeSpec::Unsatisfiable => {
            let mut res = ApiError::new(StatusCode::RANGE_NOT_SATISFIABLE, "range_not_satisfiable").into_response();
            res.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{size}")).unwrap());
            return Ok(res);
        }
    };

    let (body, len) = if size == 0 { (Body::empty(), 0) } else { (s.send_body(dev, &rec, file, start, end).await?, end + 1 - start) };

    let mut res = Response::new(body);
    *res.status_mut() = status;
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP_DOWNLOAD));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(&rec.sha256) {
        h.insert(HeaderName::from_static("x-content-sha256"), v);
    }
    if let Ok(v) = HeaderValue::from_str(&content_disposition(&rec.name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}")) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    Ok(res)
}
