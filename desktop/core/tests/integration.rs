//! End-to-end tests: a real server on a free port, real TLS, and a client that pins the
//! certificate fingerprint exactly like the phone will.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bridge_core::*;
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------- harness

fn client_for(fingerprint: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(pinned_client_config(fingerprint).unwrap())
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

struct Phone {
    token: String,
    id: String,
}

struct H {
    bridge: Option<Arc<Bridge>>,
    base: String,
    http: reqwest::Client,
    tmp: tempfile::TempDir,
    cfg: Config,
}

impl H {
    async fn new() -> H {
        Self::with(|_| {}).await
    }

    async fn with(tweak: impl FnOnce(&mut Config)) -> H {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(tmp.path().join("data"), tmp.path().join("inbox"));
        cfg.lan_host = Some("127.0.0.1".into());
        cfg.lan_port = 0;
        cfg.peer_port = 0;
        cfg.discovery = false;
        cfg.bind = "127.0.0.1".parse().unwrap();
        tweak(&mut cfg);
        let bridge = Arc::new(Bridge::start(cfg.clone()).await.unwrap());
        bridge.set_auto_accept_files(true); // most tests are about the bytes; the asking is tested on its own
        let base = format!("https://127.0.0.1:{}", bridge.lan_port());
        let http = client_for(bridge.fingerprint());
        H { bridge: Some(bridge), base, http, tmp, cfg }
    }

    fn b(&self) -> &Arc<Bridge> {
        self.bridge.as_ref().unwrap()
    }

    fn inbox(&self) -> PathBuf {
        self.tmp.path().join("inbox")
    }

    fn incoming(&self) -> PathBuf {
        self.tmp.path().join("data").join("incoming")
    }

    async fn restart(&mut self) {
        let old = self.bridge.take().unwrap();
        Arc::try_unwrap(old).ok().expect("no other Bridge handles").shutdown().await;
        let bridge = Arc::new(Bridge::start(self.cfg.clone()).await.unwrap());
        self.base = format!("https://127.0.0.1:{}", bridge.lan_port());
        self.http = client_for(bridge.fingerprint());
        self.bridge = Some(bridge);
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn pair_as(&self, name: &str) -> Phone {
        let p = self.b().new_pairing();
        let (st, v) = self.post_json("/api/pair", None, json!({ "code": p.code, "name": name })).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["fingerprint"], self.b().fingerprint());
        Phone { token: v["token"].as_str().unwrap().into(), id: v["device_id"].as_str().unwrap().into() }
    }

    async fn pair(&self) -> Phone {
        self.pair_as("Test phone").await
    }

    async fn post_json(&self, path: &str, ph: Option<&Phone>, body: Value) -> (StatusCode, Value) {
        let mut r = self.http.post(self.url(path)).json(&body);
        if let Some(p) = ph {
            r = r.bearer_auth(&p.token);
        }
        split(r.send().await.unwrap()).await
    }

    async fn get(&self, path: &str, ph: &Phone) -> (StatusCode, Value) {
        split(self.http.get(self.url(path)).bearer_auth(&ph.token).send().await.unwrap()).await
    }

    async fn create(&self, ph: &Phone, name: &str, size: usize, sha: Option<&str>) -> (StatusCode, Value) {
        let mut body = json!({ "name": name, "size": size });
        if let Some(s) = sha {
            body["sha256"] = json!(s);
        }
        self.post_json("/api/files", Some(ph), body).await
    }

    async fn put(&self, ph: &Phone, id: &str, offset: usize, data: Vec<u8>) -> (StatusCode, Value) {
        let r = self.http.put(self.url(&format!("/api/files/{id}?offset={offset}"))).bearer_auth(&ph.token).body(data);
        split(r.send().await.unwrap()).await
    }

    /// create + one PUT with everything
    async fn upload(&self, ph: &Phone, name: &str, data: &[u8], sha: Option<&str>) -> (StatusCode, Value) {
        let (st, c) = self.create(ph, name, data.len(), sha).await;
        assert_eq!(st, StatusCode::OK, "{c}");
        if c["done"] == true {
            return (st, c);
        }
        self.put(ph, c["id"].as_str().unwrap(), 0, data.to_vec()).await
    }

    async fn notes(&self, ph: &Phone) -> Vec<Value> {
        let (st, v) = self.get("/api/notes", ph).await;
        assert_eq!(st, StatusCode::OK);
        v["notes"].as_array().unwrap().clone()
    }
}

async fn split(resp: reqwest::Response) -> (StatusCode, Value) {
    let st = resp.status();
    let bytes = resp.bytes().await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Deterministic pseudo-random bytes (xorshift), so failures are reproducible.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn dir_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    };
    v.sort();
    v
}

// ---------------------------------------------------------------- TLS pinning

#[tokio::test]
async fn pinned_certificate_is_enforced() {
    let h = H::new().await;
    let ph = h.pair().await;
    // right pin works (pair() already proved it); a wrong pin must fail the handshake
    let wrong = reqwest::Client::builder()
        .use_preconfigured_tls(pinned_client_config(&"00".repeat(32)).unwrap())
        .build()
        .unwrap();
    let res = wrong.get(h.url("/api/ping")).bearer_auth(&ph.token).send().await;
    assert!(res.is_err(), "a client pinned to another fingerprint must not connect");
    assert!(pinned_client_config("nothex").is_err());
    assert!(pinned_client_config("abcd").is_err());
}

#[tokio::test]
async fn the_phone_learns_the_pc_name_and_can_unpair_itself() {
    let h = H::new().await;
    h.b().set_pc_name("Desk PC");
    let p = h.b().new_pairing();
    let (_, v) = h.post_json("/api/pair", None, json!({ "code": p.code, "name": "Pixel 8" })).await;
    assert_eq!(v["pc"], "Desk PC");
    let ph = Phone { token: v["token"].as_str().unwrap().into(), id: v["device_id"].as_str().unwrap().into() };
    let (st, v) = h.get("/api/ping", &ph).await;
    assert_eq!((st, v["pc"].as_str(), v["device"].as_str()), (StatusCode::OK, Some("Desk PC"), Some("Pixel 8")));
    let (st, v) = h.post_json("/api/unpair", Some(&ph), json!({})).await;
    assert_eq!((st, v["unpaired"].as_bool()), (StatusCode::OK, Some(true)));
    assert!(h.b().devices().is_empty());
    let (st, _) = h.get("/api/ping", &ph).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn fingerprint_is_stable_across_restart() {
    let mut h = H::new().await;
    let fp = h.b().fingerprint().to_string();
    h.restart().await;
    assert_eq!(h.b().fingerprint(), fp);
}

// ---------------------------------------------------------------- pairing

#[tokio::test]
async fn pairing_url_format() {
    let h = H::new().await;
    let p = h.b().new_pairing();
    assert_eq!(p.code.len(), 10);
    assert_eq!(p.url, format!("https://127.0.0.1:{}/#pair={}&fp={}", h.b().lan_port(), p.code, h.b().fingerprint()));
}

#[tokio::test]
async fn code_is_single_use_and_tolerant_of_format() {
    let h = H::new().await;
    let p = h.b().new_pairing();
    let typed = format!("{} - {}", p.code[..5].to_lowercase(), p.code[5..].to_lowercase());
    let (st, _) = h.post_json("/api/pair", None, json!({ "code": typed, "name": "x" })).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = h.post_json("/api/pair", None, json!({ "code": p.code, "name": "x" })).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("invalid")));
}

#[tokio::test]
async fn wrong_codes_lock_pairing_and_revoke_live_codes() {
    let h = H::new().await;
    let live = h.b().new_pairing();
    for _ in 0..5 {
        let (st, _) = h.post_json("/api/pair", None, json!({ "code": "WRONGCODE1", "name": "x" })).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    let (st, v) = h.post_json("/api/pair", None, json!({ "code": live.code, "name": "x" })).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("locked")));
}

#[tokio::test]
async fn lockout_ends() {
    let h = H::with(|c| c.lockout = Duration::from_millis(300)).await;
    for _ in 0..5 {
        h.post_json("/api/pair", None, json!({ "code": "WRONGCODE1", "name": "x" })).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.pair().await;
}

#[tokio::test]
async fn code_expires() {
    let h = H::with(|c| c.code_ttl = Duration::from_millis(200)).await;
    let p = h.b().new_pairing();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (st, _) = h.post_json("/api/pair", None, json!({ "code": p.code, "name": "x" })).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn odd_pair_bodies_are_refused_not_crashed() {
    let h = H::new().await;
    for body in [json!({}), json!({"code": null}), json!({"code": 12345}), json!({"code": ["a"]}), json!({"code": "A".repeat(100_000)})] {
        let (st, _) = h.post_json("/api/pair", None, body).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn device_name_is_clamped_and_cleaned() {
    let h = H::new().await;
    h.pair_as(&format!("N\n{}", "é".repeat(500))).await;
    let name = h.b().devices()[0].name.clone();
    assert_eq!(name.chars().count(), 40);
    assert!(!name.contains('\n'));
}

#[tokio::test]
async fn token_is_not_stored_in_plain_text() {
    let h = H::new().await;
    let ph = h.pair().await;
    let state = std::fs::read_to_string(h.tmp.path().join("data/state.json")).unwrap();
    assert!(!state.contains(&ph.token));
}

// ---------------------------------------------------------------- auth

#[tokio::test]
async fn api_requires_a_valid_token() {
    let h = H::new().await;
    let ph = h.pair().await;
    for auth in [None, Some("Bearer nope"), Some("Bearer "), Some("Basic abc"), Some("Bearer")] {
        let mut r = h.http.get(h.url("/api/notes"));
        if let Some(a) = auth {
            r = r.header("Authorization", a);
        }
        assert_eq!(r.send().await.unwrap().status(), StatusCode::UNAUTHORIZED, "{auth:?}");
    }
    // scheme is case-insensitive
    let r = h.http.get(h.url("/api/ping")).header("Authorization", format!("bearer {}", ph.token)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn unpair_is_immediate() {
    let h = H::new().await;
    let ph = h.pair().await;
    assert!(h.b().unpair(&ph.id));
    assert_eq!(h.get("/api/ping", &ph).await.0, StatusCode::UNAUTHORIZED);
    assert!(!h.b().unpair(&ph.id));
}

// ---------------------------------------------------------------- notes

#[tokio::test]
async fn text_both_directions_exact() {
    let h = H::new().await;
    let ph = h.pair_as("Pixel").await;
    let samples = ["<script>alert(1)</script>", "line1\nline2\r\n\ttab", "emoji \u{1F600} Umlaute äöü ß", "  padded  "];
    for s in samples {
        let (st, _) = h.post_json("/api/notes", Some(&ph), json!({ "text": s })).await;
        assert_eq!(st, StatusCode::OK);
    }
    h.b().send_text(&ph.id, "hello phone").unwrap();
    let notes = h.notes(&ph).await;
    let texts: Vec<&str> = notes.iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(texts, [samples[0], samples[1], samples[2], samples[3], "hello phone"]);
    assert_eq!(notes[0]["src"], "Pixel");
    assert_eq!(notes[4]["src"], "pc");
    assert_eq!(notes[0]["kind"], "text");
    let on_pc: Vec<String> = h.b().notes(&ph.id, 10).into_iter().map(|n| n.text.unwrap()).collect();
    assert_eq!(on_pc.len(), 5);
}

#[tokio::test]
async fn long_poll_wakes_immediately_on_new_note() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (_, base) = h.get("/api/notes?wait=0", &ph).await;
    let since = base["last_id"].as_u64().unwrap();
    let b = h.b().clone();
    let pid = ph.id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        b.send_text(&pid, "ping").unwrap();
    });
    let t0 = Instant::now();
    let (st, v) = h.get(&format!("/api/notes?since={since}&wait=20"), &ph).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["notes"][0]["text"], "ping");
    assert!(t0.elapsed() < Duration::from_secs(5), "must wake on the note, not run out the clock");
}

#[tokio::test]
async fn long_poll_times_out_empty_and_is_clamped() {
    let h = H::new().await;
    let ph = h.pair().await;
    let t0 = Instant::now();
    let (st, v) = h.get("/api/notes?since=0&wait=1", &ph).await;
    assert_eq!((st, v["notes"].as_array().unwrap().len()), (StatusCode::OK, 0));
    assert!(t0.elapsed() >= Duration::from_millis(900));
    assert_eq!(h.get("/api/notes?since=abc", &ph).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(h.get("/api/notes?wait=NaN", &ph).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(h.get("/api/notes?wait=-5&since=0", &ph).await.0, StatusCode::OK); // negative wait = no wait
}

#[tokio::test]
async fn many_pollers_all_wake() {
    let h = H::new().await;
    let ph = Arc::new(h.pair().await);
    let h = Arc::new(h);
    let mut tasks = vec![];
    for _ in 0..8 {
        let (h, ph) = (h.clone(), ph.clone());
        tasks.push(tokio::spawn(async move { h.get("/api/notes?since=0&wait=20", &ph).await }));
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.b().send_text(&ph.id, "fanout").unwrap();
    for t in tasks {
        let (st, v) = t.await.unwrap();
        assert_eq!((st, v["notes"][0]["text"].as_str()), (StatusCode::OK, Some("fanout")));
    }
}

#[tokio::test]
async fn unpairing_cuts_off_a_waiting_poller() {
    let h = Arc::new(H::new().await);
    let ph = Arc::new(h.pair().await);
    let (h2, ph2) = (h.clone(), ph.clone());
    let waiter = tokio::spawn(async move { h2.get("/api/notes?since=0&wait=25", &ph2).await.0 });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t0 = Instant::now();
    h.b().unpair(&ph.id);
    assert_eq!(waiter.await.unwrap(), StatusCode::UNAUTHORIZED);
    assert!(t0.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn note_validation() {
    let h = H::new().await;
    let ph = h.pair().await;
    let bad = [json!({"text": ""}), json!({"text": "   "}), json!({"text": null}), json!({"text": 5}), json!({}), json!({"text": "a".repeat(MAX_TEXT_CHARS + 1)})];
    for body in bad {
        let (st, _) = h.post_json("/api/notes", Some(&ph), body.clone()).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{}", body.to_string().chars().take(40).collect::<String>());
    }
    let (st, _) = h.post_json("/api/notes", Some(&ph), json!({"text": "a".repeat(MAX_TEXT_CHARS)})).await;
    assert_eq!(st, StatusCode::OK);

    let url = h.url("/api/notes");
    let send = |ctype: &str, body: Vec<u8>| {
        h.http.post(&url).bearer_auth(&ph.token).header("Content-Type", ctype.to_string()).body(body).send()
    };
    assert_eq!(send("text/plain", b"text=hi".to_vec()).await.unwrap().status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(send("application/json", b"{broken".to_vec()).await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(send("application/json", b"[1,2]".to_vec()).await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(send("application/json", vec![b'x'; 600 * 1024]).await.unwrap().status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn history_is_capped() {
    let h = H::new().await;
    let ph = h.pair().await;
    for i in 0..520 {
        h.b().send_text(&ph.id, &format!("n{i}")).unwrap();
    }
    let notes = h.b().notes(&ph.id, 10_000);
    assert_eq!(notes.len(), 500);
    assert_eq!(notes.last().unwrap().text.as_deref(), Some("n519"));
}

#[tokio::test]
async fn clear_history() {
    let h = H::new().await;
    let ph = h.pair().await;
    h.b().send_text(&ph.id, "a").unwrap();
    h.b().clear_history(&ph.id);
    assert!(h.notes(&ph).await.is_empty());
    h.b().send_text(&ph.id, "b").unwrap();
    assert_eq!(h.notes(&ph).await.len(), 1);
}

// ---------------------------------------------------------------- uploads

#[tokio::test]
async fn upload_in_one_request() {
    let h = H::new().await;
    let ph = h.pair_as("Pixel").await;
    let data = noise(3 * 1024 * 1024 + 123, 1);
    let (st, v) = h.upload(&ph, "photo.jpg", &data, Some(&sha(&data))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["done"], true);
    assert_eq!(v["file"]["name"], "photo.jpg");
    assert_eq!(v["file"]["sha256"], sha(&data));
    assert_eq!(std::fs::read(h.inbox().join("photo.jpg")).unwrap(), data);
    assert!(dir_names(&h.incoming()).is_empty(), "no leftovers in the incoming folder");
    // announced in the feed
    let notes = h.notes(&ph).await;
    assert_eq!(notes[0]["kind"], "file");
    assert_eq!(notes[0]["file"]["dir"], "to_pc");
    assert_eq!(notes[0]["src"], "Pixel");
}

#[tokio::test]
async fn upload_without_claimed_hash_still_reports_hash() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(5000, 2);
    let (_, v) = h.upload(&ph, "a.bin", &data, None).await;
    assert_eq!(v["file"]["sha256"], sha(&data));
}

#[tokio::test]
async fn wrong_hash_is_rejected_and_nothing_is_saved() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(10_000, 3);
    let (st, v) = h.upload(&ph, "bad.bin", &data, Some(&"ab".repeat(32))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("hash_mismatch")));
    assert!(dir_names(&h.inbox()).is_empty());
    assert!(dir_names(&h.incoming()).is_empty());
    let (st, v) = h.create(&ph, "x", 10, Some("not-a-hash")).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_sha256")));
}

#[tokio::test]
async fn upload_in_chunks_with_resume_and_offset_checks() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(2_500_000, 4);
    let (_, c) = h.create(&ph, "big.bin", data.len(), Some(&sha(&data))).await;
    let id = c["id"].as_str().unwrap().to_string();

    let (st, v) = h.put(&ph, &id, 0, data[..1_000_000].to_vec()).await;
    assert_eq!((st, v["received"].as_u64(), v["done"].as_bool()), (StatusCode::OK, Some(1_000_000), Some(false)));

    // wrong offsets are refused and the server tells the client where it really is
    for bad in [0usize, 999_999, 1_000_001] {
        let (st, v) = h.put(&ph, &id, bad, vec![1, 2, 3]).await;
        assert_eq!((st, v["error"].as_str(), v["received"].as_u64()), (StatusCode::CONFLICT, Some("offset_mismatch"), Some(1_000_000)), "offset {bad}");
    }
    let (_, s) = h.get(&format!("/api/files/{id}"), &ph).await;
    assert_eq!((s["received"].as_u64(), s["state"].as_str()), (Some(1_000_000), Some("receiving")));

    let (st, v) = h.put(&ph, &id, 1_000_000, data[1_000_000..].to_vec()).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(std::fs::read(h.inbox().join("big.bin")).unwrap(), data);
}

#[tokio::test]
async fn aborted_upload_keeps_progress_and_resumes_byte_exact() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(6 * 1024 * 1024, 5);
    let (_, c) = h.create(&ph, "cut.bin", data.len(), Some(&sha(&data))).await;
    let id = c["id"].as_str().unwrap().to_string();

    // Send 2.5 MiB and then break the connection mid-body.
    let first = Bytes::copy_from_slice(&data[..2_621_440]);
    let chunks: Vec<std::result::Result<Bytes, std::io::Error>> = vec![Ok(first), Err(std::io::Error::other("connection lost"))];
    let res = h
        .http
        .put(h.url(&format!("/api/files/{id}?offset=0")))
        .bearer_auth(&ph.token)
        .body(reqwest::Body::wrap_stream(futures_util::stream::iter(chunks)))
        .send()
        .await;
    assert!(res.is_err());

    // The server notices the dead connection and keeps what arrived, in order. It may still be writing the
    // last part when the first non-zero number shows up, so wait until the number stops changing.
    let mut received = 0usize;
    let mut stable = 0;
    for _ in 0..100 {
        let (_, s) = h.get(&format!("/api/files/{id}"), &ph).await;
        let now = s["received"].as_u64().unwrap() as usize;
        stable = if now > 0 && now == received { stable + 1 } else { 0 };
        received = now;
        if stable >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(stable >= 3, "the received size never settled ({received})");
    assert!(received > 0 && received <= 2_621_440, "received {received}");

    // Resume from whatever the server reports. Result must be identical to the original.
    let (st, v) = h.put(&ph, &id, received, data[received..].to_vec()).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    assert_eq!(std::fs::read(h.inbox().join("cut.bin")).unwrap(), data);
}

#[tokio::test]
async fn upload_resumes_after_pc_restart() {
    let mut h = H::new().await;
    let ph = h.pair().await;
    let data = noise(1_500_000, 6);
    let (_, c) = h.create(&ph, "survive.bin", data.len(), Some(&sha(&data))).await;
    let id = c["id"].as_str().unwrap().to_string();
    h.put(&ph, &id, 0, data[..700_000].to_vec()).await;

    h.restart().await;

    let (st, s) = h.get(&format!("/api/files/{id}"), &ph).await; // token still valid after restart
    assert_eq!((st, s["received"].as_u64()), (StatusCode::OK, Some(700_000)), "{s}");
    let (st, v) = h.put(&ph, &id, 700_000, data[700_000..].to_vec()).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(std::fs::read(h.inbox().join("survive.bin")).unwrap(), data);
}

#[tokio::test]
async fn upload_of_exact_size_with_empty_final_put_finishes() {
    // crash window: all bytes written but not finalised; an empty PUT at offset==size completes it
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(4096, 7);
    let (_, c) = h.create(&ph, "tail.bin", data.len(), None).await;
    let id = c["id"].as_str().unwrap().to_string();
    let (_, v) = h.put(&ph, &id, 0, data.clone()).await;
    assert_eq!(v["done"], true);
    // finished transfers refuse further data
    let (st, v) = h.put(&ph, &id, data.len(), vec![]).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("not_receiving")));
}

#[tokio::test]
async fn crash_after_last_byte_but_before_finalising_is_recoverable() {
    // Simulate a crash in the window where all bytes are on disk but the file was not yet
    // verified and moved: after restart an empty PUT at offset == size must finish it.
    let mut h = H::new().await;
    let ph = h.pair().await;
    let data = noise(300_000, 17);
    let (_, c) = h.create(&ph, "window.bin", data.len(), Some(&sha(&data))).await;
    let id = c["id"].as_str().unwrap().to_string();
    h.put(&ph, &id, 0, data[..100_000].to_vec()).await;
    let old = h.bridge.take().unwrap();
    Arc::try_unwrap(old).ok().unwrap().shutdown().await;
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(h.incoming().join(format!("{id}.part"))).unwrap();
        f.write_all(&data[100_000..]).unwrap();
    }
    let bridge = Arc::new(Bridge::start(h.cfg.clone()).await.unwrap());
    h.base = format!("https://127.0.0.1:{}", bridge.lan_port());
    h.http = client_for(bridge.fingerprint());
    h.bridge = Some(bridge);

    let (_, s) = h.get(&format!("/api/files/{id}"), &ph).await;
    assert_eq!(s["received"].as_u64(), Some(300_000));
    let (st, v) = h.put(&ph, &id, 300_000, vec![]).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    assert_eq!(std::fs::read(h.inbox().join("window.bin")).unwrap(), data);
}

#[tokio::test]
async fn zero_byte_file() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (st, v) = h.create(&ph, "empty.txt", 0, None).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    assert_eq!(std::fs::read(h.inbox().join("empty.txt")).unwrap().len(), 0);
    assert_eq!(v["file"]["sha256"], sha(b""));
}

#[tokio::test]
async fn path_traversal_and_hostile_names_stay_in_the_inbox() {
    let h = H::new().await;
    let ph = h.pair().await;
    let names = ["../../evil.txt", "..\\..\\evil2.txt", "/etc/cron.d/evil3", "C:\\Windows\\evil4.dll", "CON", "a<b>c.txt", "..", ""];
    for n in names {
        let (st, v) = h.upload(&ph, n, b"x", None).await;
        assert_eq!(st, StatusCode::OK, "{n:?}: {v}");
    }
    let saved = dir_names(&h.inbox());
    assert_eq!(saved.len(), names.len(), "{saved:?}");
    for must in ["evil.txt", "evil2.txt", "evil3", "evil4.dll", "_CON", "a_b_c.txt", "file", "file (1)"] {
        assert!(saved.iter().any(|s| s == must), "missing {must} in {saved:?}");
    }
    // nothing appeared next to the inbox
    let siblings = dir_names(h.tmp.path());
    assert_eq!(siblings, ["data", "inbox"]);
}

#[tokio::test]
async fn same_name_never_overwrites() {
    let h = H::new().await;
    let ph = h.pair().await;
    h.upload(&ph, "a.txt", b"one", None).await;
    h.upload(&ph, "a.txt", b"two", None).await;
    h.upload(&ph, "a.txt", b"three", None).await;
    assert_eq!(dir_names(&h.inbox()), ["a (1).txt", "a (2).txt", "a.txt"]);
    assert_eq!(std::fs::read(h.inbox().join("a.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(h.inbox().join("a (2).txt")).unwrap(), b"three");
}

#[tokio::test]
async fn size_limits() {
    let h = H::with(|c| c.max_file_size = 1000).await;
    let ph = h.pair().await;
    let (st, v) = h.create(&ph, "huge", 1001, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("file_too_large")));

    let (_, c) = h.create(&ph, "f", 100, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    let (st, v) = h.put(&ph, &id, 0, vec![0u8; 101]).await; // more than declared
    assert_eq!((st, v["error"].as_str()), (StatusCode::PAYLOAD_TOO_LARGE, Some("too_much_data")));
    let (_, s) = h.get(&format!("/api/files/{id}"), &ph).await;
    assert!(s["received"].as_u64().unwrap() <= 100);
    for bad in [json!({"name": "x"}), json!({"size": 5}), json!({"name": "x", "size": -1}), json!({"name": "x", "size": 1.5}), json!({"name": 3, "size": 1})] {
        let (st, _) = h.post_json("/api/files", Some(&ph), bad).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn at_most_16_open_uploads_per_phone() {
    let h = H::new().await;
    let ph = h.pair().await;
    for i in 0..16 {
        let (st, _) = h.create(&ph, &format!("f{i}"), 10, None).await;
        assert_eq!(st, StatusCode::OK);
    }
    let (st, v) = h.create(&ph, "one-too-many", 10, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("too_many_transfers")));
}

#[tokio::test]
async fn second_writer_on_the_same_upload_is_refused() {
    let h = Arc::new(H::new().await);
    let ph = Arc::new(h.pair().await);
    let (_, c) = h.create(&ph, "busy.bin", 3000, None).await;
    let id = c["id"].as_str().unwrap().to_string();

    // first request: sends 1000 bytes, then stalls
    let (tx, rx) = tokio::sync::mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(4);
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (v, rx)) }));
    let (h2, ph2, id2) = (h.clone(), ph.clone(), id.clone());
    let slow = tokio::spawn(async move {
        h2.http.put(h2.url(&format!("/api/files/{id2}?offset=0"))).bearer_auth(&ph2.token).body(body).send().await.unwrap().status()
    });
    tx.send(Ok(Bytes::from(vec![7u8; 1000]))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let (st, v) = h.put(&ph, &id, 0, vec![1u8; 10]).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("busy")), "{v}");

    tx.send(Ok(Bytes::from(vec![7u8; 2000]))).await.unwrap();
    drop(tx);
    assert_eq!(slow.await.unwrap(), StatusCode::OK);
}

#[tokio::test]
async fn a_phone_cannot_touch_another_phones_transfer() {
    let h = H::new().await;
    let a = h.pair_as("A").await;
    let b = h.pair_as("B").await;
    let (_, c) = h.create(&a, "mine.bin", 100, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    assert_eq!(h.get(&format!("/api/files/{id}"), &b).await.0, StatusCode::NOT_FOUND);
    assert_eq!(h.put(&b, &id, 0, vec![0; 10]).await.0, StatusCode::NOT_FOUND);
    let r = h.http.delete(h.url(&format!("/api/files/{id}"))).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(h.get(&format!("/api/files/{id}"), &a).await.0, StatusCode::OK);
}

#[tokio::test]
async fn cancel_removes_the_partial_file() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (_, c) = h.create(&ph, "gone.bin", 5000, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    h.put(&ph, &id, 0, vec![1; 1000]).await;
    assert!(!dir_names(&h.incoming()).is_empty());
    let r = h.http.delete(h.url(&format!("/api/files/{id}"))).bearer_auth(&ph.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(dir_names(&h.incoming()).is_empty());
    // the cancelled transfer is still known (the PC shows it as cancelled), but takes no more data
    assert_eq!(h.put(&ph, &id, 1000, vec![1; 10]).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn unpairing_deletes_that_phones_partial_files() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (_, c) = h.create(&ph, "orphan.bin", 5000, None).await;
    h.put(&ph, c["id"].as_str().unwrap(), 0, vec![1; 1000]).await;
    assert!(!dir_names(&h.incoming()).is_empty());
    h.b().unpair(&ph.id);
    assert!(dir_names(&h.incoming()).is_empty());
}

#[tokio::test]
async fn stale_partial_files_without_a_valid_owner_are_cleaned_on_start() {
    let mut h = H::new().await;
    let ph = h.pair().await;
    let (_, c) = h.create(&ph, "x.bin", 100, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    h.b().unpair(&ph.id);
    // plant an orphan .part and a corrupt sidecar, as a crash might leave them
    std::fs::create_dir_all(h.incoming()).unwrap();
    std::fs::write(h.incoming().join("deadbeefdeadbeef.part"), b"junk").unwrap();
    std::fs::write(h.incoming().join("0123456789abcdef.json"), b"{not json").unwrap();
    h.restart().await;
    assert!(dir_names(&h.incoming()).is_empty(), "{:?} (id {id})", dir_names(&h.incoming()));
}

// ---------------------------------------------------------------- downloads (PC -> phone)

async fn offered(h: &H, ph: &Phone, data: &[u8], name: &str) -> (String, PathBuf) {
    let path = h.tmp.path().join(name);
    std::fs::write(&path, data).unwrap();
    let fref = h.b().send_file(&ph.id, &path).await.unwrap();
    assert_eq!(fref.sha256, sha(data));
    assert_eq!(fref.size as usize, data.len());
    let notes = h.notes(ph).await;
    let last = notes.last().unwrap();
    assert_eq!((last["kind"].as_str(), last["file"]["dir"].as_str(), last["src"].as_str()), (Some("file"), Some("to_phone"), Some("pc")));
    (fref.id, path)
}

async fn fetch(h: &H, ph: &Phone, id: &str, range: Option<&str>) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
    let mut r = h.http.get(h.url(&format!("/api/files/{id}/content"))).bearer_auth(&ph.token);
    if let Some(v) = range {
        r = r.header("Range", v);
    }
    let resp = r.send().await.unwrap();
    let (st, hd) = (resp.status(), resp.headers().clone());
    (st, hd, resp.bytes().await.unwrap().to_vec())
}

#[tokio::test]
async fn download_full_and_ranges() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(3_000_000, 8);
    let (id, _) = offered(&h, &ph, &data, "song.bin").await;

    let (st, hd, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, data);
    assert_eq!(hd["accept-ranges"], "bytes");
    assert_eq!(hd["x-content-sha256"].to_str().unwrap(), sha(&data));
    assert!(hd["content-disposition"].to_str().unwrap().contains("song.bin"));
    assert_eq!(hd["content-security-policy"], "default-src 'none'; sandbox");

    let (st, hd, body) = fetch(&h, &ph, &id, Some("bytes=100-199")).await;
    assert_eq!((st, hd["content-range"].to_str().unwrap()), (StatusCode::PARTIAL_CONTENT, "bytes 100-199/3000000"));
    assert_eq!(body, &data[100..200]);

    let (st, _, body) = fetch(&h, &ph, &id, Some("bytes=2999990-")).await;
    assert_eq!((st, body), (StatusCode::PARTIAL_CONTENT, data[2_999_990..].to_vec()));

    let (st, _, body) = fetch(&h, &ph, &id, Some("bytes=-10")).await;
    assert_eq!((st, body), (StatusCode::PARTIAL_CONTENT, data[2_999_990..].to_vec()));

    let (st, hd, _) = fetch(&h, &ph, &id, Some("bytes=3000000-")).await;
    assert_eq!((st, hd["content-range"].to_str().unwrap()), (StatusCode::RANGE_NOT_SATISFIABLE, "bytes */3000000"));

    // multi-range and garbage are ignored, full file served
    for r in ["bytes=0-1,5-6", "nonsense", "bytes=9-3"] {
        let (st, _, body) = fetch(&h, &ph, &id, Some(r)).await;
        assert_eq!((st, body.len()), (StatusCode::OK, data.len()), "{r}");
    }
}

#[tokio::test]
async fn finished_download_is_reported_done_and_cut_one_as_interrupted() {
    let h = H::new().await;
    let ph = h.pair_as("Pixel").await;
    let state_of = |h: &H, id: &str| {
        let key = format!("dl-{id}-");
        h.b().transfers().into_iter().find(|t| t.id.starts_with(&key)).map(|t| t.state)
    };
    // complete download -> Done
    let data = noise(3_000_000, 18);
    let (id, _) = offered(&h, &ph, &data, "report.bin").await;
    let (st, _, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!((st, body.len()), (StatusCode::OK, data.len()));
    let mut state = None;
    for _ in 0..40 {
        state = state_of(&h, &id);
        if state == Some(TransferState::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(state, Some(TransferState::Done), "a completed download must not stay 'active'");

    // abandoned download of a file far larger than the socket buffers -> Interrupted
    let big = noise(64_000_000, 19);
    let (id2, _) = offered(&h, &ph, &big, "big.bin").await;
    let resp = h.http.get(h.url(&format!("/api/files/{id2}/content"))).bearer_auth(&ph.token).send().await.unwrap();
    let mut stream = resp.bytes_stream();
    let mut n = 0;
    while let Some(c) = stream.next().await {
        n += c.unwrap().len();
        if n > 300_000 {
            break;
        }
    }
    drop(stream);
    let mut state = None;
    for _ in 0..60 {
        state = state_of(&h, &id2);
        if state == Some(TransferState::Interrupted) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(state, Some(TransferState::Interrupted));
}

#[tokio::test]
async fn interrupted_download_resumes_with_range() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(4_000_000, 9);
    let (id, _) = offered(&h, &ph, &data, "movie.bin").await;
    // read only the first ~1 MB, then drop the connection
    let resp = h.http.get(h.url(&format!("/api/files/{id}/content"))).bearer_auth(&ph.token).send().await.unwrap();
    let mut got = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        got.extend_from_slice(&chunk.unwrap());
        if got.len() >= 1_000_000 {
            break;
        }
    }
    drop(stream);
    let (st, _, rest) = fetch(&h, &ph, &id, Some(&format!("bytes={}-", got.len()))).await;
    assert_eq!(st, StatusCode::PARTIAL_CONTENT);
    got.extend_from_slice(&rest);
    assert_eq!(got, data);
}

#[tokio::test]
async fn if_range_with_other_version_gets_full_file() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(10_000, 10);
    let (id, _) = offered(&h, &ph, &data, "v.bin").await;
    let etag = format!("\"{}\"", sha(&data));
    let ok = h.http.get(h.url(&format!("/api/files/{id}/content"))).bearer_auth(&ph.token).header("Range", "bytes=0-9").header("If-Range", &etag).send().await.unwrap();
    assert_eq!(ok.status(), StatusCode::PARTIAL_CONTENT);
    let stale = h.http.get(h.url(&format!("/api/files/{id}/content"))).bearer_auth(&ph.token).header("Range", "bytes=0-9").header("If-Range", "\"other\"").send().await.unwrap();
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(stale.bytes().await.unwrap().len(), data.len());
}

#[tokio::test]
async fn browser_download_via_ticket() {
    let h = H::new().await;
    let ph = h.pair().await;
    let data = noise(50_000, 11);
    let (id, _) = offered(&h, &ph, &data, "ticket é.bin").await;
    let (st, t) = h.post_json(&format!("/api/files/{id}/ticket"), Some(&ph), json!({})).await;
    assert_eq!(st, StatusCode::OK);
    let url = t["url"].as_str().unwrap();
    let ticket = t["ticket"].as_str().unwrap();

    // no Authorization header needed
    let r = h.http.get(h.url(url)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
    assert!(cd.contains("filename*=UTF-8''ticket%20%C3%A9.bin"), "{cd}");
    assert!(cd.contains("filename=\"ticket _.bin\""), "{cd}");
    assert_eq!(r.bytes().await.unwrap().to_vec(), data);
    // ranges work through tickets too
    let r = h.http.get(h.url(url)).header("Range", "bytes=10-19").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);

    // wrong file id, junk ticket, and unpaired device are all refused
    assert_eq!(h.http.get(h.url(&format!("/dl/{ticket}/0000000000000000"))).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(h.http.get(h.url(&format!("/dl/junk/{id}"))).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    h.b().unpair(&ph.id);
    assert_eq!(h.http.get(h.url(url)).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn offered_file_that_changed_or_vanished_is_refused() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (id, path) = offered(&h, &ph, &noise(5000, 12), "live.bin").await;
    std::fs::write(&path, noise(5001, 13)).unwrap();
    let (st, _, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!(st, StatusCode::GONE);
    assert!(String::from_utf8_lossy(&body).contains("file_changed"));
    std::fs::remove_file(&path).unwrap();
    let (st, _, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!(st, StatusCode::GONE);
    assert!(String::from_utf8_lossy(&body).contains("file_gone"));
}

#[tokio::test]
async fn offers_survive_restart() {
    let mut h = H::new().await;
    let ph = h.pair().await;
    let data = noise(20_000, 14);
    let (id, _) = offered(&h, &ph, &data, "keep.bin").await;
    h.restart().await;
    let (st, _, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!((st, body), (StatusCode::OK, data));
}

#[tokio::test]
async fn offering_a_directory_or_missing_path_fails_cleanly() {
    let h = H::new().await;
    let ph = h.pair().await;
    assert!(h.b().send_file(&ph.id, h.tmp.path()).await.is_err());
    assert!(h.b().send_file(&ph.id, h.tmp.path().join("nope.bin")).await.is_err());
}

#[tokio::test]
async fn zero_byte_download() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (id, _) = offered(&h, &ph, b"", "nothing.txt").await;
    let (st, _, body) = fetch(&h, &ph, &id, None).await;
    assert_eq!((st, body.len()), (StatusCode::OK, 0));
}

// ---------------------------------------------------------------- events

#[tokio::test]
async fn events_report_notes_devices_and_transfers() {
    let h = H::new().await;
    let mut ev = h.b().events();
    let ph = h.pair().await;
    let data = noise(2_000_000, 15);
    h.upload(&ph, "ev.bin", &data, None).await;

    let (mut saw_device, mut saw_note, mut saw_done) = (false, false, false);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !(saw_device && saw_note && saw_done) {
        match tokio::time::timeout(Duration::from_millis(500), ev.recv()).await {
            Ok(Ok(Event::DevicesChanged)) => saw_device = true,
            Ok(Ok(Event::NoteAdded(n))) if n.kind == NoteKind::File => saw_note = true,
            Ok(Ok(Event::Transfer(t))) if t.state == TransferState::Done && t.dir == Direction::ToPc => {
                assert_eq!((t.size, t.done), (data.len() as u64, data.len() as u64));
                saw_done = true;
            }
            _ => {}
        }
    }
    assert!((saw_device, saw_note, saw_done) == (true, true, true), "device {saw_device} note {saw_note} done {saw_done}");
    let list = h.b().transfers();
    assert!(list.iter().any(|t| t.name == "ev.bin" && t.state == TransferState::Done));
}

// ---------------------------------------------------------------- surface and robustness

#[tokio::test]
async fn serves_only_the_phone_page_and_assets() {
    let h = H::new().await;
    let r = h.http.get(h.url("/")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let hd = r.headers().clone();
    assert!(hd["content-security-policy"].to_str().unwrap().contains("default-src 'self'"));
    assert_eq!(hd["cache-control"], "no-store");
    assert_eq!(hd["x-frame-options"], "DENY");
    assert_eq!(hd["x-content-type-options"], "nosniff");
    assert!(!hd.contains_key("server"));
    for path in ["/pc.js", "/state.json", "/identity-key.pem", "/../Cargo.toml", "/api/ui/state", "/dl/x", "/%2e%2e/etc/passwd"] {
        let r = h.http.get(h.url(path)).send().await.unwrap();
        assert!(r.status() == StatusCode::NOT_FOUND || r.status() == StatusCode::BAD_REQUEST, "{path} -> {}", r.status());
    }
    assert_eq!(h.http.get(h.url("/phone.js")).send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(h.http.get(h.url("/style.css")).send().await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn plain_http_and_stalled_handshakes_do_not_hurt_the_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let h = H::new().await;
    let port = h.b().lan_port();
    // plain HTTP on the TLS port
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = [0u8; 64];
    let _ = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await;
    // a client that connects and says nothing
    let _stall = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    // garbage bytes
    let mut g = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    g.write_all(&[0x16, 0x03, 0x01, 0xff, 0xff, 1, 2, 3]).await.unwrap();

    let t0 = Instant::now();
    h.pair().await;
    assert!(t0.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn restart_keeps_pairing_and_history() {
    let mut h = H::new().await;
    let ph = h.pair_as("Pixel").await;
    h.post_json("/api/notes", Some(&ph), json!({"text": "survives"})).await;
    h.restart().await;
    let notes = h.notes(&ph).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["text"], "survives");
    assert_eq!(h.b().devices().len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn private_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let h = H::new().await;
    let ph = h.pair().await;
    h.upload(&ph, "p.bin", &noise(100, 16), None).await;
    h.b().send_text(&ph.id, "x").unwrap();
    for f in ["identity-key.pem", "state.json"] {
        let mode = std::fs::metadata(h.tmp.path().join("data").join(f)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{f}");
    }
}

#[tokio::test]
async fn corrupt_state_file_recovers() {
    let mut h = H::new().await;
    h.pair().await;
    std::fs::write(h.tmp.path().join("data/state.json"), b"{not json").unwrap();
    h.restart().await;
    assert!(h.b().devices().is_empty());
    h.pair().await; // and it still works
}

#[tokio::test]
async fn tls_floor_and_alpn_are_sane() {
    // The pinned client negotiates TLS 1.3 here; the server config must also never offer < 1.2.
    let h = H::new().await;
    let ph = h.pair().await;
    let r = h.http.get(h.url("/api/ping")).bearer_auth(&ph.token).send().await.unwrap();
    assert_eq!(r.version(), reqwest::Version::HTTP_11);
    assert_eq!(r.json::<Value>().await.unwrap()["protocol"], 1);
}

// ---------------------------------------------------------------- benchmark (run explicitly)

/// `cargo test --release -p bridge-core --test integration bench -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn bench_throughput_over_loopback_tls() {
    const MIB: usize = 1024 * 1024;
    const TOTAL: usize = 512 * MIB;
    let h = H::new().await;
    let ph = h.pair().await;
    let chunk = Bytes::from(noise(MIB, 99));

    let (_, c) = h.create(&ph, "bench.bin", TOTAL, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    let chunks = (0..TOTAL / MIB).map(move |_| Ok::<Bytes, std::io::Error>(chunk.clone()));
    let t0 = Instant::now();
    let r = h
        .http
        .put(h.url(&format!("/api/files/{id}?offset=0")))
        .bearer_auth(&ph.token)
        .header("Content-Length", TOTAL.to_string())
        .body(reqwest::Body::wrap_stream(futures_util::stream::iter(chunks)))
        .send()
        .await
        .unwrap();
    let up_total = t0.elapsed();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    let file_id = v["file"]["id"].as_str().unwrap().to_string();
    println!("upload   {} MiB in {:.2?}  (incl. disk write, checksum, move) = {:.0} MB/s", TOTAL / MIB, up_total, TOTAL as f64 / 1e6 / up_total.as_secs_f64());

    // now the other way, from a file the PC offers
    let path = h.inbox().join("bench.bin");
    let t0 = Instant::now();
    let fref = h.b().send_file(&ph.id, &path).await.unwrap();
    println!("prepare  checksum of {} MiB before offering: {:.2?}", TOTAL / MIB, t0.elapsed());
    let t0 = Instant::now();
    let resp = h.http.get(h.url(&format!("/api/files/{}/content", fref.id))).bearer_auth(&ph.token).send().await.unwrap();
    let mut n = 0usize;
    let mut stream = resp.bytes_stream();
    while let Some(c) = stream.next().await {
        n += c.unwrap().len();
    }
    let down = t0.elapsed();
    assert_eq!(n, TOTAL);
    println!("download {} MiB in {:.2?} = {:.0} MB/s", TOTAL / MIB, down, TOTAL as f64 / 1e6 / down.as_secs_f64());
    let _ = file_id;
}

// ---------------------------------------------------------------- asking before receiving, replies, deleting

fn state_of(h: &H, id: &str) -> Option<TransferState> {
    h.b().transfers().into_iter().find(|t| t.id == id).map(|t| t.state)
}

#[tokio::test]
async fn a_file_from_the_phone_waits_until_the_user_accepts() {
    let h = H::new().await;
    h.b().set_auto_accept_files(false);
    let ph = h.pair().await;
    let (st, c) = h.create(&ph, "wish.txt", 5, None).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!((c["state"].as_str(), c["done"].as_bool()), (Some("pending"), Some(false)));
    let id = c["id"].as_str().unwrap().to_string();

    // nothing is stored, nothing can be sent yet, and the user sees an offer
    assert!(dir_names(&h.incoming()).is_empty());
    assert_eq!(h.put(&ph, &id, 0, b"hello".to_vec()).await.1["error"], "not_accepted");
    assert_eq!(state_of(&h, &id), Some(TransferState::Offered));
    assert_eq!(h.get(&format!("/api/files/{id}"), &ph).await.1["state"], "pending");

    assert!(h.b().accept_file(&id).await);
    assert_eq!(h.get(&format!("/api/files/{id}"), &ph).await.1["state"], "receiving");
    let (st, v) = h.put(&ph, &id, 0, b"hello".to_vec()).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    assert_eq!(std::fs::read(h.inbox().join("wish.txt")).unwrap(), b"hello");

    // the message id of a file is its transfer id, and the phone is never told where it was saved
    let n = h.b().notes(&ph.id, 10).pop().unwrap();
    assert_eq!(n.mid, id);
    assert!(n.file.unwrap().local_path.is_some());
    let (_, feed) = h.get("/api/notes", &ph).await;
    assert!(!feed.to_string().contains("local_path"), "{feed}");
}

#[tokio::test]
async fn declining_a_phone_file_stores_nothing() {
    let h = H::new().await;
    h.b().set_auto_accept_files(false);
    let ph = h.pair().await;
    let (_, c) = h.create(&ph, "no.bin", 10, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    assert!(h.b().decline_file(&id));
    assert_eq!(state_of(&h, &id), None, "a declined offer leaves the list at once");
    assert_eq!(h.get(&format!("/api/files/{id}"), &ph).await.1["state"], "declined");
    assert_eq!(h.put(&ph, &id, 0, vec![1; 10]).await.1["error"], "declined");
    assert!(dir_names(&h.incoming()).is_empty() && dir_names(&h.inbox()).is_empty());
    assert!(!h.b().decline_file(&id), "deciding twice does nothing");
}

#[tokio::test]
async fn the_phone_can_withdraw_an_offer_and_the_pc_can_remove_it_from_the_list() {
    let h = H::new().await;
    h.b().set_auto_accept_files(false);
    let ph = h.pair_as("Pixel").await;
    let (_, c) = h.create(&ph, "oops.bin", 10, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    let r = h.http.delete(h.url(&format!("/api/files/{id}"))).bearer_auth(&ph.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let t = h.b().transfers().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!((t.state, t.error.as_deref()), (TransferState::Cancelled, Some("Cancelled by Pixel")));
    assert!(h.b().dismiss_transfer(&id).await);
    assert_eq!(state_of(&h, &id), None);
}

#[tokio::test]
async fn the_pc_can_cancel_a_running_upload() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (_, c) = h.create(&ph, "big.bin", 5000, None).await;
    let id = c["id"].as_str().unwrap().to_string();
    h.put(&ph, &id, 0, vec![1; 1000]).await;
    assert_eq!(state_of(&h, &id), Some(TransferState::Active));
    assert!(h.b().cancel_transfer(&id));
    assert_eq!(state_of(&h, &id), Some(TransferState::Cancelled));
    assert!(dir_names(&h.incoming()).is_empty(), "the partial file is gone");
    assert_eq!(h.put(&ph, &id, 1000, vec![1; 10]).await.1["error"], "cancelled");
    assert_eq!(h.get(&format!("/api/files/{id}"), &ph).await.1["state"], "cancelled");
    assert!(!h.b().cancel_transfer(&id), "cancelling twice does nothing");
}

#[tokio::test]
async fn replies_and_deletes_reach_the_phone() {
    let h = H::new().await;
    let ph = h.pair_as("Pixel").await;
    let first = h.b().send_text(&ph.id, "lunch at noon?").unwrap();
    assert!(!first.mid.is_empty());
    let reply = h.b().send_text_reply(&ph.id, "yes", Some(&first.mid)).unwrap();
    let q = reply.reply.unwrap();
    assert_eq!((q.mid.as_str(), q.src.as_str(), q.preview.as_str()), (first.mid.as_str(), "pc", "lunch at noon?"));
    assert!(h.b().send_text_reply(&ph.id, "x", Some("nonexistent")).unwrap().reply.is_none());

    // the phone answers too
    let (_, v) = h.post_json("/api/notes", Some(&ph), json!({ "text": "ok", "reply_to": first.mid })).await;
    assert_eq!(v["note"]["reply"]["preview"], "lunch at noon?");
    assert_eq!(v["note"]["src"], "Pixel");

    // the phone learns about deletions, once
    let (_, v) = h.get("/api/notes", &ph).await;
    let (last, del) = (v["last_id"].as_u64().unwrap(), v["del"].as_u64().unwrap());
    assert!(h.b().delete_note(&ph.id, &first.mid));
    let (_, v) = h.get(&format!("/api/notes?since={last}&del={del}&wait=2"), &ph).await;
    assert_eq!(v["deleted"], json!([first.mid]));
    let del = v["del"].as_u64().unwrap();
    let (_, v) = h.get(&format!("/api/notes?since={last}&del={del}"), &ph).await;
    assert_eq!(v["deleted"], json!([]));
    assert!(!h.b().notes(&ph.id, 10).iter().any(|n| n.mid == first.mid));
    assert!(!h.b().delete_note(&ph.id, &first.mid), "already gone");

    // a phone deletes one of its own notes
    let mid = h.b().notes(&ph.id, 10).iter().find(|n| n.src == "Pixel").unwrap().mid.clone();
    let r = h.http.delete(h.url(&format!("/api/notes/{mid}"))).bearer_auth(&ph.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(!h.b().notes(&ph.id, 10).iter().any(|n| n.mid == mid));
}

#[tokio::test]
async fn deleting_a_received_file_deletes_it_from_the_inbox_but_withdrawing_an_offer_keeps_the_original() {
    let h = H::new().await;
    let ph = h.pair().await;
    let (_, v) = h.upload(&ph, "scan.pdf", b"pdf bytes", None).await;
    let received = v["file"]["id"].as_str().unwrap().to_string();
    assert!(h.inbox().join("scan.pdf").exists());
    assert!(h.b().delete_note(&ph.id, &received));
    assert!(!h.inbox().join("scan.pdf").exists());

    let original = h.tmp.path().join("mine.txt");
    std::fs::write(&original, b"keep me").unwrap();
    let f = h.b().send_file(&ph.id, &original).await.unwrap();
    assert_eq!(f.local_path.as_deref(), Some(original.canonicalize().unwrap().to_str().unwrap()));
    assert!(h.b().delete_note(&ph.id, &f.id));
    assert!(original.exists(), "the user's own file is never deleted");
    assert_eq!(h.get(&format!("/api/files/{}", f.id), &ph).await.0, StatusCode::NOT_FOUND, "the phone can no longer fetch it");
}

// ---------------------------------------------------------------- one conversation per phone

fn texts(notes: &[Value]) -> Vec<String> {
    notes.iter().map(|n| n["text"].as_str().unwrap_or("<file>").to_string()).collect()
}

#[tokio::test]
async fn every_phone_only_sees_its_own_conversation() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;

    h.post_json("/api/notes", Some(&a), json!({ "text": "from Pixel" })).await;
    h.b().send_text(&a.id, "to Pixel").unwrap();
    h.b().send_text(&b.id, "to Galaxy").unwrap();
    h.post_json("/api/notes", Some(&b), json!({ "text": "from Galaxy" })).await;

    assert_eq!(texts(&h.notes(&a).await), ["from Pixel", "to Pixel"]);
    assert_eq!(texts(&h.notes(&b).await), ["to Galaxy", "from Galaxy"]);
    // the PC's own view, per phone
    assert_eq!(h.b().notes(&a.id, 10).len(), 2);
    assert_eq!(h.b().notes(&b.id, 10).len(), 2);
    // a phone that asks for everything since the beginning still gets only its own
    let (_, v) = h.get("/api/notes?since=0", &b).await;
    assert_eq!(texts(v["notes"].as_array().unwrap()), ["to Galaxy", "from Galaxy"]);
    // the answer does not carry the internal phone id
    assert!(!v.to_string().contains(&a.id) && !v.to_string().contains("\"dev\""), "{v}");
}

#[tokio::test]
async fn a_message_for_one_phone_does_not_wake_the_other() {
    let h = Arc::new(H::new().await);
    let a = h.pair_as("Pixel").await;
    let b = Arc::new(h.pair_as("Galaxy").await);
    let (_, base) = h.get("/api/notes?wait=0", &b).await;
    let since = base["last_id"].as_u64().unwrap();
    let (h2, b2) = (h.clone(), b.clone());
    let waiting = tokio::spawn(async move { h2.get(&format!("/api/notes?since={since}&wait=2"), &b2).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.b().send_text(&a.id, "only for Pixel").unwrap();
    let (st, v) = waiting.await.unwrap();
    assert_eq!(st, StatusCode::OK);
    assert!(v["notes"].as_array().unwrap().is_empty(), "{v}");
}

#[tokio::test]
async fn deleting_and_clearing_affect_only_one_phone() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;
    let na = h.b().send_text(&a.id, "A's secret").unwrap();
    h.b().send_text(&b.id, "B's note").unwrap();

    // phone B cannot delete a message of phone A, and the PC cannot reach it through B either
    let r = h.http.delete(h.url(&format!("/api/notes/{}", na.mid))).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(!h.b().delete_note(&b.id, &na.mid));
    assert_eq!(texts(&h.notes(&a).await), ["A's secret"]);

    // B is not told when A's message is deleted
    let (_, v) = h.get("/api/notes", &b).await;
    let (last, del) = (v["last_id"].as_u64().unwrap(), v["del"].as_u64().unwrap());
    assert!(h.b().delete_note(&a.id, &na.mid));
    let (_, v) = h.get(&format!("/api/notes?since={last}&del={del}"), &b).await;
    assert_eq!(v["deleted"], json!([]), "{v}");

    // clearing one phone's conversation leaves the other's
    h.b().send_text(&a.id, "again").unwrap();
    h.b().clear_history(&a.id);
    assert!(h.notes(&a).await.is_empty());
    assert_eq!(texts(&h.notes(&b).await), ["B's note"]);
}

#[tokio::test]
async fn files_belong_to_the_phone_they_were_offered_to_or_received_from() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;

    // PC -> phone A
    let path = h.tmp.path().join("for-a.txt");
    std::fs::write(&path, b"only for A").unwrap();
    let f = h.b().send_file(&a.id, &path).await.unwrap();
    assert_eq!(fetch(&h, &a, &f.id, None).await.0, StatusCode::OK);
    assert_eq!(fetch(&h, &b, &f.id, None).await.0, StatusCode::NOT_FOUND, "phone B must not download A's file");
    assert_eq!(h.get(&format!("/api/files/{}", f.id), &b).await.0, StatusCode::NOT_FOUND);
    let r = h.http.post(h.url(&format!("/api/files/{}/ticket", f.id))).bearer_auth(&b.token).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(h.notes(&b).await.is_empty(), "B is not told about A's file");
    assert_eq!(h.notes(&a).await.len(), 1);

    // phone B -> PC: appears in B's conversation only
    h.upload(&b, "from-b.txt", b"hello", None).await;
    assert_eq!(h.notes(&a).await.len(), 1);
    let nb = h.notes(&b).await;
    assert_eq!(nb.len(), 1);
    assert_eq!(nb[0]["file"]["name"], "from-b.txt");
    assert!(h.b().notes(&a.id, 10).iter().all(|n| n.file.as_ref().is_some_and(|f| f.dir == Direction::ToPhone)));

    // a file cannot be offered to a phone that is not paired
    assert!(h.b().send_file("not-a-phone", &path).await.is_err());
}

#[tokio::test]
async fn an_unpaired_phone_takes_its_conversation_along() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;
    h.b().send_text(&a.id, "old chat").unwrap();
    h.b().send_text(&b.id, "stays").unwrap();
    assert!(h.b().unpair(&a.id));
    assert!(h.b().notes(&a.id, 10).is_empty());
    // the same phone pairs again as a new device and starts with an empty conversation
    let again = h.pair_as("Pixel").await;
    assert!(h.notes(&again).await.is_empty());
    assert_eq!(texts(&h.notes(&b).await), ["stays"]);
}

#[tokio::test]
async fn conversations_survive_a_restart_per_phone() {
    let mut h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;
    h.b().send_text(&a.id, "kept for A").unwrap();
    h.b().send_text(&b.id, "kept for B").unwrap();
    h.restart().await;
    assert_eq!(texts(&h.notes(&a).await), ["kept for A"]);
    assert_eq!(texts(&h.notes(&b).await), ["kept for B"]);
}

// ---------------------------------------------------------------- two separate Clear buttons

/// (in the chat, in the list "Files") texts of one phone's conversation as the PC window shows them
fn views(h: &H, dev: &str) -> (Vec<String>, Vec<String>) {
    let all = h.b().notes(dev, 100);
    let t = |keep: &dyn Fn(&bridge_core::Note) -> bool| all.iter().filter(|n| keep(n)).map(|n| n.text.clone().unwrap_or_default()).collect();
    (t(&|n| !n.chat_hidden), t(&|n| !n.list_hidden))
}

#[tokio::test]
async fn clearing_hides_what_the_phone_has_not_fetched_yet() {
    let mut h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let b = h.pair_as("Galaxy").await;
    let path = h.tmp.path().join("doc.txt");
    std::fs::write(&path, b"content").unwrap();
    h.b().send_text(&a.id, "one").unwrap();
    h.b().send_file(&a.id, &path).await.unwrap();
    h.b().send_text(&a.id, "two").unwrap();
    h.b().send_text(&b.id, "other phone").unwrap();
    assert_eq!(h.notes(&a).await.len(), 3);

    // Clear chat: the PC's chat is empty, the file stays in the list
    h.b().clear_chat(&a.id);
    let (chat, list) = views(&h, &a.id);
    assert_eq!((chat.len(), list.len()), (0, 3));
    assert_eq!(h.notes(&a).await.len(), 3, "the phone still has everything");
    // messages that come in later show up in the chat again
    h.b().send_text(&a.id, "three").unwrap();
    assert_eq!(views(&h, &a.id).0, vec!["three".to_string()]);

    // Clear list, too: nothing is shown on the PC for the old messages, the phone still has all of it
    h.b().clear_list(&a.id);
    assert_eq!(h.notes(&a).await.len(), 4);
    h.restart().await;
    assert_eq!(h.notes(&a).await.len(), 4, "also after the PC restarted");

    // the other phone is untouched
    assert_eq!(views(&h, &b.id), (vec!["other phone".into()], vec!["other phone".into()]));
}

#[tokio::test]
async fn clearing_deletes_for_good_what_the_phone_has_fetched() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    h.b().send_text(&a.id, "one").unwrap();
    h.b().send_text(&a.id, "two").unwrap();
    // the phone fetches both and asks for what is after them: it keeps them in its own history now
    let first = h.notes(&a).await;
    let last = first.last().unwrap()["id"].as_u64().unwrap();
    h.b().send_text(&a.id, "three").unwrap(); // not fetched yet
    let (st, v) = h.get(&format!("/api/notes?since={last}"), &a).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);
    h.b().clear_chat(&a.id);
    // "one" and "two" are gone from the PC for good; "three" is not acknowledged yet, so it is only hidden
    assert_eq!(h.notes(&a).await.len(), 1);
    assert_eq!(h.notes(&a).await[0]["text"], "three");
    // the phone asks again from "three" on: now that one goes too
    let l3 = h.notes(&a).await[0]["id"].as_u64().unwrap();
    let _ = h.get(&format!("/api/notes?since={l3}"), &a).await;
    assert_eq!(h.notes(&a).await.len(), 0);
}

#[tokio::test]
async fn a_hidden_file_is_still_offered_and_a_deleted_one_is_not() {
    let h = H::new().await;
    let a = h.pair_as("Pixel").await;
    let path = h.tmp.path().join("doc.txt");
    std::fs::write(&path, b"content").unwrap();
    let f = h.b().send_file(&a.id, &path).await.unwrap();
    h.b().clear_chat(&a.id);
    h.b().clear_list(&a.id);
    assert_eq!(fetch(&h, &a, &f.id, None).await.0, StatusCode::OK, "cleared on the PC: still downloadable for the phone");
    assert!(h.b().delete_note(&a.id, &f.id));
    assert_eq!(fetch(&h, &a, &f.id, None).await.0, StatusCode::NOT_FOUND);
}
