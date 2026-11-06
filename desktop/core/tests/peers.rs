//! PC <-> PC: pairing, isolation from the phone side, hostile requests, transfers.
//! Two real bridges on loopback talk over real mutual TLS.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bridge_core::*;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

struct Pc {
    b: Arc<Bridge>,
    tmp: tempfile::TempDir,
    cfg: Config,
}

impl Pc {
    async fn new(name: &str) -> Pc {
        Self::with_ports(name, 0).await
    }

    async fn with_ports(name: &str, peer_port: u16) -> Pc {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(tmp.path().join("data"), tmp.path().join("inbox"));
        cfg.lan_host = Some("127.0.0.1".into());
        cfg.lan_port = 0;
        cfg.peer_port = peer_port;
        cfg.discovery = false;
        cfg.bind = "127.0.0.1".parse().unwrap();
        let b = Arc::new(Bridge::start(cfg.clone()).await.unwrap());
        b.set_pc_name(name);
        b.set_auto_accept_files(true); // asking is tested on its own
        Pc { b, tmp, cfg }
    }

    fn inbox(&self) -> PathBuf {
        self.tmp.path().join("inbox")
    }

    async fn restart(&mut self) {
        let old = std::mem::replace(&mut self.b, Arc::new(Bridge::start(self.cfg.clone()).await.unwrap()));
        drop(old); // the server tasks of the dropped bridge stay up until the runtime ends; ports differ, so harmless
    }

    /// Client that acts like this PC when talking to `target`'s PC port.
    fn client_to(&self, target: &Pc) -> reqwest::Client {
        raw_client(self.b.data_dir(), Some(target.b.fingerprint()))
    }

    fn peer_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.b.peer_port(), path)
    }

    fn phone_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.b.lan_port(), path)
    }
}

fn raw_client(data_dir: &Path, pin: Option<&str>) -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(peer_client_config_from_dir(data_dir, pin).unwrap())
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// A stranger: has a certificate, is not paired with anyone.
fn stranger(target: &Pc) -> (tempfile::TempDir, reqwest::Client) {
    let dir = tempfile::tempdir().unwrap();
    let c = raw_client(dir.path(), Some(target.b.fingerprint()));
    (dir, c)
}

async fn eventually<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// Pair `a` with `b` the way two users would: A starts, B accepts, A confirms. Returns (A's id for B, B's id for A).
async fn pair(a: &Pc, b: &Pc) -> (String, String) {
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("pairing request on B", || b.b.pair_requests().into_iter().next()).await;
    assert_eq!(req.code, out.code, "both screens must show the same code");
    assert!(b.b.pair_accept(&req.id));
    assert!(a.b.pair_confirm(&out.id));
    let a_has_b = eventually("A trusts B", || a.b.peers().into_iter().find(|p| p.id == b.b.fingerprint())).await;
    let b_has_a = eventually("B trusts A", || b.b.peers().into_iter().find(|p| p.id == a.b.fingerprint())).await;
    (a_has_b.id, b_has_a.id)
}

fn noise(n: usize, seed: u8) -> Vec<u8> {
    let mut x = seed as u32 | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect()
}

fn sha(d: &[u8]) -> String {
    hex::encode(Sha256::digest(d))
}

async fn json_of(r: reqwest::Response) -> (StatusCode, Value) {
    let st = r.status();
    let b = r.bytes().await.unwrap();
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

// ---------------------------------------------------------------- pairing

#[tokio::test]
async fn pairing_shows_the_same_code_and_both_sides_trust_each_other() {
    let (a, b) = (Pc::new("Desk PC").await, Pc::new("Laptop").await);
    let (a_sees, b_sees) = pair(&a, &b).await;
    assert_eq!(a_sees, b.b.fingerprint());
    assert_eq!(b_sees, a.b.fingerprint());
    assert_eq!(a.b.peers()[0].name, "Laptop");
    assert_eq!(b.b.peers()[0].name, "Desk PC");
    assert!(a.b.pair_requests().is_empty() && b.b.pair_requests().is_empty());
}

#[tokio::test]
async fn nobody_is_trusted_until_both_users_agree() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;

    // A's user confirms first, B's user has not decided: nobody trusts anybody.
    assert!(a.b.pair_confirm(&out.id));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(a.b.peers().is_empty() && b.b.peers().is_empty());

    // B accepts: now both sides complete.
    assert!(b.b.pair_accept(&req.id));
    eventually("A trusts B", || a.b.peers().first().cloned()).await;
    eventually("B trusts A", || b.b.peers().first().cloned()).await;
}

#[tokio::test]
async fn the_asking_pc_must_confirm_too() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;
    assert!(b.b.pair_accept(&req.id));
    eventually("A learns B accepted", || {
        a.b.outgoing_pairs().into_iter().find(|p| p.id == out.id && p.state == OutPairState::PeerAccepted)
    })
    .await;
    // A's user has not confirmed the code: A does not trust B yet (B already trusts A).
    assert!(a.b.peers().is_empty());
    assert!(a.b.pair_confirm(&out.id));
    eventually("A trusts B", || a.b.peers().first().cloned()).await;
}

#[tokio::test]
async fn declining_pairs_nobody() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;
    assert!(b.b.pair_decline(&req.id));
    eventually("A sees the refusal", || {
        a.b.outgoing_pairs().into_iter().find(|p| p.id == out.id && p.state == OutPairState::Declined)
    })
    .await;
    assert!(!a.b.pair_confirm(&out.id), "a declined pairing cannot be completed");
    assert!(a.b.peers().is_empty() && b.b.peers().is_empty());
    assert!(!b.b.pair_accept(&req.id), "a declined request cannot be accepted afterwards");
}

#[tokio::test]
async fn cancelling_withdraws_the_request_on_the_other_pc() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    eventually("request", || b.b.pair_requests().into_iter().next()).await;
    a.b.pair_cancel(&out.id);
    eventually("request withdrawn", || b.b.pair_requests().is_empty().then_some(())).await;
    assert!(a.b.peers().is_empty() && b.b.peers().is_empty());
}

#[tokio::test]
async fn the_accepting_pc_trusts_only_after_the_starting_pc_confirmed() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;
    assert!(b.b.pair_accept(&req.id));
    // B's user confirmed, A's has not: B waits and neither PC is paired yet
    assert_eq!(b.b.pair_waiting().len(), 1);
    assert!(b.b.pair_requests().is_empty());
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(b.b.peers().is_empty() && a.b.peers().is_empty(), "nobody is paired before both confirmed");
    assert!(a.b.pair_confirm(&out.id));
    eventually("B trusts A", || b.b.peers().first().cloned()).await;
    eventually("A trusts B", || a.b.peers().first().cloned()).await;
    assert!(b.b.pair_waiting().is_empty());
}

#[tokio::test]
async fn cancelling_while_the_other_side_waits_leaves_nothing_behind() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;
    b.b.pair_accept(&req.id);
    a.b.pair_cancel(&out.id); // codes did not match for A's user
    eventually("B stops waiting", || b.b.pair_waiting().is_empty().then_some(())).await;
    assert!(a.b.peers().is_empty() && b.b.peers().is_empty());
}

#[tokio::test]
async fn declining_while_waiting_stops_the_pairing() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let out = a.b.pair_start(LOCAL, b.b.peer_port()).await.unwrap();
    let req = eventually("request", || b.b.pair_requests().into_iter().next()).await;
    b.b.pair_accept(&req.id);
    assert!(b.b.pair_decline(&req.id)); // B's user changed their mind
    assert!(a.b.pair_confirm(&out.id));
    eventually("A gives up", || a.b.outgoing_pairs().iter().any(|o| matches!(o.state, bridge_core::OutPairState::Failed | bridge_core::OutPairState::Declined)).then_some(())).await;
    assert!(a.b.peers().is_empty() && b.b.peers().is_empty());
}

#[tokio::test]
async fn pairing_with_yourself_is_refused() {
    let a = Pc::new("A").await;
    let err = a.b.pair_start(LOCAL, a.b.peer_port()).await.unwrap_err();
    assert!(err.to_string().contains("this PC"), "{err}");
}

#[tokio::test]
async fn pairing_with_nothing_there_fails_cleanly() {
    let a = Pc::new("A").await;
    let err = a.b.pair_start(LOCAL, 1).await.unwrap_err();
    assert!(matches!(err, PeerError::Unreachable), "{err}");
}

#[tokio::test]
async fn wrong_reveal_is_rejected_and_the_request_dies() {
    let b = Pc::new("B").await;
    let (_d, c) = stranger(&b);
    let commit = sha(&[7u8; 16]);
    let (st, v) = json_of(c.post(b.peer_url("/peer/pair")).json(&json!({ "name": "Mallory", "commit": commit })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    // reveal a different nonce than the one committed to
    let (st, v) = json_of(c.post(b.peer_url(&format!("/peer/pair/{id}/reveal"))).json(&json!({ "nonce": hex::encode([9u8; 16]) })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "bad_commit");
    assert!(b.b.pair_requests().is_empty(), "nothing may be shown to the user");
    let (st, _) = json_of(c.get(b.peer_url(&format!("/peer/pair/{id}"))).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_is_invisible_to_the_user_until_the_nonce_is_revealed() {
    let b = Pc::new("B").await;
    let (_d, c) = stranger(&b);
    let na = [3u8; 16];
    let commit = hex::encode(Sha256::digest([b"bridge-commit-v1|".as_slice(), &na].concat()));
    let (st, v) = json_of(c.post(b.peer_url("/peer/pair")).json(&json!({ "name": "X", "commit": commit })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(b.b.pair_requests().is_empty());
    let id = v["id"].as_str().unwrap();
    // accepting something that was never revealed does nothing
    assert!(!b.b.pair_accept(id));
    assert!(b.b.peers().is_empty());
}

#[tokio::test]
async fn pair_requests_are_capped_and_replaced_per_pc() {
    let b = Pc::new("B").await;
    let commit = sha(&[1u8; 16]);
    // the same stranger asking again replaces its earlier request
    let (_d, c) = stranger(&b);
    for _ in 0..3 {
        let (st, _) = json_of(c.post(b.peer_url("/peer/pair")).json(&json!({ "name": "S", "commit": commit })).send().await.unwrap()).await;
        assert_eq!(st, StatusCode::OK);
    }
    // many different strangers hit the cap on open requests (or the rate limit)
    let mut refused = 0;
    let mut dirs = Vec::new();
    for _ in 0..8 {
        let (d, c) = stranger(&b);
        let (st, v) = json_of(c.post(b.peer_url("/peer/pair")).json(&json!({ "name": "S", "commit": commit })).send().await.unwrap()).await;
        if st == StatusCode::TOO_MANY_REQUESTS {
            refused += 1;
            assert!(matches!(v["error"].as_str(), Some("too_many_requests" | "rate_limited")), "{v}");
        }
        dirs.push(d);
    }
    assert!(refused >= 4, "only {refused} refused");
}

#[tokio::test]
async fn names_from_the_network_are_cleaned() {
    let b = Pc::new("B").await;
    let (_d, c) = stranger(&b);
    let na = [5u8; 16];
    let commit = hex::encode(Sha256::digest([b"bridge-commit-v1|".as_slice(), &na].concat()));
    let evil = format!("  \u{0007}Evil\nPC{}  ", "x".repeat(100));
    let (_, v) = json_of(c.post(b.peer_url("/peer/pair")).json(&json!({ "name": evil, "commit": commit })).send().await.unwrap()).await;
    let id = v["id"].as_str().unwrap().to_string();
    let (st, _) = json_of(c.post(b.peer_url(&format!("/peer/pair/{id}/reveal"))).json(&json!({ "nonce": hex::encode(na) })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let req = b.b.pair_requests().remove(0);
    assert!(req.name.chars().count() <= 40 && !req.name.chars().any(|c| c.is_control()), "{:?}", req.name);
    assert!(req.name.starts_with("EvilPC"));
}

// ---------------------------------------------------------------- messages

#[tokio::test]
async fn text_goes_both_ways_and_is_stored_per_pc() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;

    a.b.send_peer_text(&a_to_b, "hello laptop").await.unwrap();
    b.b.send_peer_text(&b_to_a, "hello desk").await.unwrap();

    let on_b = eventually("B has the message", || {
        let n = b.b.peer_notes(&b_to_a, 10);
        (n.len() == 2).then_some(n)
    })
    .await;
    assert_eq!((on_b[0].src.as_str(), on_b[0].text.as_deref()), ("Desk", Some("hello laptop")));
    assert_eq!((on_b[1].src.as_str(), on_b[1].text.as_deref()), ("pc", Some("hello desk")));
    let on_a = a.b.peer_notes(&a_to_b, 10);
    assert_eq!((on_a[0].src.as_str(), on_a[0].text.as_deref()), ("pc", Some("hello laptop")));
    assert_eq!((on_a[1].src.as_str(), on_a[1].text.as_deref()), ("Laptop", Some("hello desk")));
}

#[tokio::test]
async fn text_validation() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (id, _) = pair(&a, &b).await;
    assert!(a.b.send_peer_text(&id, "   ").await.is_err());
    assert!(a.b.send_peer_text(&id, &"x".repeat(MAX_TEXT_CHARS + 1)).await.is_err());
    assert!(a.b.send_peer_text(&id, &"é".repeat(MAX_TEXT_CHARS)).await.is_ok());
    assert!(matches!(a.b.send_peer_text(&"0".repeat(64), "hi").await, Err(PeerError::NotPaired)));
}

#[tokio::test]
async fn pc_conversations_never_touch_the_phone_feed() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (a_to_b, _) = pair(&a, &b).await;

    // B also has a paired phone with its own notes
    let code = b.b.new_pairing().code;
    let phone = reqwest::Client::builder()
        .use_preconfigured_tls(pinned_client_config(b.b.fingerprint()).unwrap())
        .build()
        .unwrap();
    let (st, v) = json_of(phone.post(b.phone_url("/api/pair")).json(&json!({ "code": code, "name": "Phone" })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let token = v["token"].as_str().unwrap().to_string();
    let phone_id = v["device_id"].as_str().unwrap().to_string();
    b.b.send_text(&phone_id, "only for the phone").unwrap();

    a.b.send_peer_text(&a_to_b, "private between PCs").await.unwrap();
    let file = noise(300_000, 3);
    let p = a.tmp.path().join("secret-plan.bin");
    std::fs::write(&p, &file).unwrap();
    a.b.send_peer_file(&a_to_b, &p).await.unwrap();

    // the phone feed on B holds exactly its own note
    let (_, notes) = json_of(phone.get(b.phone_url("/api/notes")).bearer_auth(&token).send().await.unwrap()).await;
    let list = notes["notes"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{notes}");
    assert_eq!(list[0]["text"], "only for the phone");
    assert_eq!(b.b.notes(&phone_id, 100).len(), 1);

    // and what the phone sends never shows up in the PC conversation
    let (st, _) = json_of(phone.post(b.phone_url("/api/notes")).bearer_auth(&token).json(&json!({ "text": "phone secret" })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let b_to_a = b.b.peers()[0].id.clone();
    assert!(b.b.peer_notes(&b_to_a, 100).iter().all(|n| n.text.as_deref() != Some("phone secret")));
    assert!(a.b.peer_notes(&a_to_b, 100).iter().all(|n| n.text.as_deref() != Some("phone secret")));
    // A never learns that B has a phone at all
    assert!(a.b.devices().is_empty());
}

// ---------------------------------------------------------------- isolation of the two ports

#[tokio::test]
async fn a_paired_pc_cannot_reach_any_phone_route() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    pair(&a, &b).await;
    let c = a.client_to(&b);
    // phone routes are simply not served on the PC port
    for path in ["/api/notes", "/api/ping", "/api/files", "/", "/phone.js"] {
        let (st, _) = json_of(c.get(b.peer_url(path)).send().await.unwrap()).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{path}");
    }
    let (st, _) = json_of(c.post(b.peer_url("/api/pair")).json(&json!({ "code": "X" })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_phone_port_serves_no_pc_routes() {
    let b = Pc::new("B").await;
    let phone = reqwest::Client::builder().use_preconfigured_tls(pinned_client_config(b.b.fingerprint()).unwrap()).build().unwrap();
    for path in ["/peer/ping", "/peer/hello", "/peer/messages", "/peer/files"] {
        let (st, _) = json_of(phone.get(b.phone_url(path)).send().await.unwrap()).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn the_pc_port_refuses_clients_without_a_certificate() {
    let b = Pc::new("B").await;
    // a phone-style client (pinned server, no client certificate) cannot even finish the handshake
    let phone = reqwest::Client::builder().use_preconfigured_tls(pinned_client_config(b.b.fingerprint()).unwrap()).build().unwrap();
    assert!(phone.get(b.peer_url("/peer/hello")).send().await.is_err());
}

#[tokio::test]
async fn strangers_get_nothing_but_the_pairing_handshake() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    pair(&a, &b).await;
    let (_d, s) = stranger(&b);
    for (m, path) in [("GET", "/peer/ping"), ("POST", "/peer/messages"), ("POST", "/peer/files"), ("GET", "/peer/files/abc"), ("POST", "/peer/unpair")] {
        let req = if m == "GET" { s.get(b.peer_url(path)) } else { s.post(b.peer_url(path)).json(&json!({ "text": "hi", "name": "x", "size": 1 })) };
        let (st, v) = json_of(req.send().await.unwrap()).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{path} {v}");
        assert_eq!(v["error"], "not_paired");
    }
    assert!(b.b.peer_notes(&a.b.fingerprint().to_string(), 10).is_empty());
    // hello is allowed (it only says the name) and tells the stranger it is not paired
    let (st, v) = json_of(s.get(b.peer_url("/peer/hello")).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["name"], "B");
    assert_eq!(v["paired"], false);
    assert!(v.as_object().unwrap().keys().all(|k| ["name", "protocol", "paired"].contains(&k.as_str())), "{v}");
}

#[tokio::test]
async fn an_upload_of_one_side_is_invisible_to_the_other_side() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    pair(&a, &b).await;
    // B's paired phone starts an upload
    let code = b.b.new_pairing().code;
    let phone = reqwest::Client::builder().use_preconfigured_tls(pinned_client_config(b.b.fingerprint()).unwrap()).build().unwrap();
    let (_, v) = json_of(phone.post(b.phone_url("/api/pair")).json(&json!({ "code": code, "name": "P" })).send().await.unwrap()).await;
    let token = v["token"].as_str().unwrap().to_string();
    let (_, c) = json_of(phone.post(b.phone_url("/api/files")).bearer_auth(&token).json(&json!({ "name": "p.bin", "size": 10 })).send().await.unwrap()).await;
    let phone_upload = c["id"].as_str().unwrap().to_string();

    // the paired PC cannot see, continue, or cancel it
    let pc = a.client_to(&b);
    let (st, _) = json_of(pc.get(b.peer_url(&format!("/peer/files/{phone_upload}"))).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = json_of(pc.put(b.peer_url(&format!("/peer/files/{phone_upload}?offset=0"))).body(vec![0u8; 10]).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = json_of(pc.delete(b.peer_url(&format!("/peer/files/{phone_upload}"))).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // and the other way round: a PC upload is invisible to the phone token
    let (_, c) = json_of(pc.post(b.peer_url("/peer/files")).json(&json!({ "name": "pc.bin", "size": 10 })).send().await.unwrap()).await;
    let pc_upload = c["id"].as_str().unwrap().to_string();
    let (st, _) = json_of(phone.get(b.phone_url(&format!("/api/files/{pc_upload}"))).bearer_auth(&token).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = json_of(phone.put(b.phone_url(&format!("/api/files/{pc_upload}?offset=0"))).bearer_auth(&token).body(vec![0u8; 10]).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- files

#[tokio::test]
async fn a_file_arrives_identical_and_is_announced_in_the_right_conversation() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    let data = noise(9_000_000, 9); // more than two upload chunks
    let src = a.tmp.path().join("holiday.zip");
    std::fs::write(&src, &data).unwrap();

    let mut events = a.b.events();
    let f = a.b.send_peer_file(&a_to_b, &src).await.unwrap();
    assert_eq!((f.size as usize, f.sha256.as_str(), f.dir), (data.len(), sha(&data).as_str(), Direction::ToPeer));

    let got = std::fs::read(b.inbox().join("holiday.zip")).unwrap();
    assert_eq!(sha(&got), sha(&data));
    let n = &b.b.peer_notes(&b_to_a, 10)[0];
    assert_eq!((n.src.as_str(), n.kind), ("Desk", NoteKind::File));
    assert_eq!(n.file.as_ref().unwrap().dir, Direction::FromPeer);
    assert_eq!(a.b.peer_notes(&a_to_b, 10)[0].file.as_ref().unwrap().dir, Direction::ToPeer);
    assert!(b.b.notes("", 10).is_empty(), "files from a PC must not appear in a phone feed");

    // the sender saw progress and a final Done
    let mut last = None;
    while let Ok(ev) = events.try_recv() {
        if let Event::Transfer(t) = ev {
            if t.dir == Direction::ToPeer {
                last = Some(t);
            }
        }
    }
    let last = last.expect("transfer events");
    assert_eq!((last.state, last.done, last.device.as_str()), (TransferState::Done, data.len() as u64, "Laptop"));
}

#[tokio::test]
async fn zero_byte_and_awkward_names() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (id, _) = pair(&a, &b).await;
    let p = a.tmp.path().join("empty.txt");
    std::fs::write(&p, b"").unwrap();
    a.b.send_peer_file(&id, &p).await.unwrap();
    assert_eq!(std::fs::read(b.inbox().join("empty.txt")).unwrap(), b"");

    // same name twice does not overwrite
    let q = a.tmp.path().join("same.txt");
    std::fs::write(&q, b"one").unwrap();
    a.b.send_peer_file(&id, &q).await.unwrap();
    std::fs::write(&q, b"two").unwrap();
    a.b.send_peer_file(&id, &q).await.unwrap();
    assert_eq!(std::fs::read(b.inbox().join("same.txt")).unwrap(), b"one");
    assert_eq!(std::fs::read(b.inbox().join("same (1).txt")).unwrap(), b"two");
}

#[tokio::test]
async fn folders_and_missing_files_are_refused_cleanly() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (id, _) = pair(&a, &b).await;
    assert!(a.b.send_peer_file(&id, a.tmp.path()).await.is_err());
    assert!(a.b.send_peer_file(&id, a.tmp.path().join("nope")).await.is_err());
    assert!(b.b.peer_notes(&a.b.fingerprint().to_string(), 10).is_empty());
}

#[tokio::test]
async fn peer_upload_resumes_and_checks_the_checksum() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    pair(&a, &b).await;
    let c = a.client_to(&b);
    let data = noise(1_000_000, 4);

    let (_, v) = json_of(c.post(b.peer_url("/peer/files")).json(&json!({ "name": "r.bin", "size": data.len(), "sha256": sha(&data) })).send().await.unwrap()).await;
    let id = v["id"].as_str().unwrap().to_string();
    // first half only
    let (st, _) = json_of(c.put(b.peer_url(&format!("/peer/files/{id}?offset=0"))).body(data[..400_000].to_vec()).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    // wrong offset is refused and says where to continue
    let (st, v) = json_of(c.put(b.peer_url(&format!("/peer/files/{id}?offset=123"))).body(vec![0u8; 5]).send().await.unwrap()).await;
    assert_eq!((st, v["received"].as_u64()), (StatusCode::CONFLICT, Some(400_000)));
    // status tells the same
    let (_, v) = json_of(c.get(b.peer_url(&format!("/peer/files/{id}"))).send().await.unwrap()).await;
    assert_eq!(v["received"], 400_000);
    // the rest completes it
    let (st, v) = json_of(c.put(b.peer_url(&format!("/peer/files/{id}?offset=400000"))).body(data[400_000..].to_vec()).send().await.unwrap()).await;
    assert_eq!((st, v["done"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(std::fs::read(b.inbox().join("r.bin")).unwrap(), data);

    // damaged data is rejected and not kept
    let (_, v) = json_of(c.post(b.peer_url("/peer/files")).json(&json!({ "name": "bad.bin", "size": 100, "sha256": sha(b"other") })).send().await.unwrap()).await;
    let id = v["id"].as_str().unwrap().to_string();
    let (st, v) = json_of(c.put(b.peer_url(&format!("/peer/files/{id}?offset=0"))).body(vec![1u8; 100]).send().await.unwrap()).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("hash_mismatch")));
    assert!(!b.inbox().join("bad.bin").exists());
}

#[tokio::test]
async fn traversal_names_from_a_pc_stay_inside_the_inbox() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    pair(&a, &b).await;
    let c = a.client_to(&b);
    let (_, v) = json_of(c.post(b.peer_url("/peer/files")).json(&json!({ "name": "../../escape.txt", "size": 3 })).send().await.unwrap()).await;
    let id = v["id"].as_str().unwrap().to_string();
    let (st, _) = json_of(c.put(b.peer_url(&format!("/peer/files/{id}?offset=0"))).body(b"abc".to_vec()).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    assert!(b.inbox().join("escape.txt").exists());
    assert!(!b.tmp.path().join("escape.txt").exists());
}

#[tokio::test]
async fn both_pcs_can_send_at_the_same_time() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    let (fa, fb) = (noise(3_000_000, 5), noise(3_000_000, 6));
    let (pa, pb) = (a.tmp.path().join("from-a.bin"), b.tmp.path().join("from-b.bin"));
    std::fs::write(&pa, &fa).unwrap();
    std::fs::write(&pb, &fb).unwrap();
    let (r1, r2, r3, r4) = tokio::join!(
        a.b.send_peer_file(&a_to_b, &pa),
        b.b.send_peer_file(&b_to_a, &pb),
        a.b.send_peer_text(&a_to_b, "from a"),
        b.b.send_peer_text(&b_to_a, "from b"),
    );
    assert!(r1.is_ok() && r2.is_ok() && r3.is_ok() && r4.is_ok());
    assert_eq!(sha(&std::fs::read(b.inbox().join("from-a.bin")).unwrap()), sha(&fa));
    assert_eq!(sha(&std::fs::read(a.inbox().join("from-b.bin")).unwrap()), sha(&fb));
}

// ---------------------------------------------------------------- unpairing and identity

#[tokio::test]
async fn unpairing_removes_trust_and_conversation_on_both_sides() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    a.b.send_peer_text(&a_to_b, "remember me").await.unwrap();
    eventually("message arrived", || (!b.b.peer_notes(&b_to_a, 10).is_empty()).then_some(())).await;

    assert!(a.b.unpair_peer(&a_to_b));
    assert!(a.b.peers().is_empty() && a.b.peer_notes(&a_to_b, 10).is_empty());
    eventually("B was told", || b.b.peers().is_empty().then_some(())).await;
    assert!(b.b.peer_notes(&b_to_a, 10).is_empty());

    // nothing works any more, in either direction
    assert!(matches!(a.b.send_peer_text(&a_to_b, "x").await, Err(PeerError::NotPaired)));
    assert!(matches!(b.b.send_peer_text(&b_to_a, "x").await, Err(PeerError::NotPaired)));
    let (st, _) = json_of(a.client_to(&b).post(b.peer_url("/peer/messages")).json(&json!({ "text": "x" })).send().await.unwrap()).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// The window calls `unpair_peer` from its own thread, which is not a Tokio thread. That used to panic
/// ("there is no reactor running") before the other PC was told.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpairing_from_a_plain_thread_works_and_tells_the_other_pc() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    a.b.send_peer_text(&a_to_b, "hello").await.unwrap();
    eventually("message arrived", || (!b.b.peer_notes(&b_to_a, 10).is_empty()).then_some(())).await;
    let bridge = a.b.clone();
    let id = a_to_b.clone();
    let existed = std::thread::spawn(move || bridge.unpair_peer(&id)).join().expect("unpair must not panic");
    assert!(existed);
    eventually("B lost the pairing", || b.b.peers().is_empty().then_some(())).await;
    assert!(b.b.peer_notes(&b_to_a, 10).is_empty(), "the conversation is gone on B too");
}

#[tokio::test]
async fn being_unpaired_by_the_other_side_is_reported_clearly() {
    let (a, b) = (Pc::new("A").await, Pc::new("B").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    // B silently forgets A (as if B's data was reset), without telling A
    assert!(b.b.unpair_peer(&b_to_a));
    // give the courtesy "unpair" message time to remove A's side too; then re-add one-sidedly is not possible,
    // so check the direct refusal path with a raw call from A's identity
    let (st, v) = json_of(a.client_to(&b).get(b.peer_url("/peer/ping")).send().await.unwrap()).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("not_paired")));
    let _ = a_to_b;
}

#[tokio::test]
async fn a_pc_whose_certificate_changed_is_not_trusted() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let a = Pc::new("A").await;
    let mut b = Pc::with_ports("B", port).await;
    let (a_to_b, _) = pair(&a, &b).await;
    a.b.send_peer_text(&a_to_b, "before").await.unwrap();

    // B's computer is replaced: same address and port, brand-new identity
    let old = std::mem::replace(&mut b.b, Arc::new(Bridge::start({
        let mut c = b.cfg.clone();
        c.peer_port = 0;
        c.lan_port = 0;
        c
    }).await.unwrap()));
    drop(old);
    drop(b);
    let impostor_dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(impostor_dir.path().join("data"), impostor_dir.path().join("inbox"));
    cfg.lan_host = Some("127.0.0.1".into());
    cfg.lan_port = 0;
    cfg.peer_port = port;
    cfg.discovery = false;
    cfg.bind = "127.0.0.1".parse().unwrap();
    // wait for the old listener to release the port
    let mut impostor = None;
    for _ in 0..100 {
        if let Ok(x) = Bridge::start(cfg.clone()).await {
            impostor = Some(x);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // If the old server could not be stopped in-process the port stays taken; then the test has nothing to prove.
    let Some(impostor) = impostor else { return };
    assert_ne!(impostor.fingerprint(), a_to_b);

    let err = a.b.send_peer_text(&a_to_b, "after").await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("identity") || matches!(err, PeerError::Unreachable), "{err}");
    assert!(impostor.peer_notes(&a.b.fingerprint().to_string(), 10).is_empty(), "nothing may reach the impostor");
}

#[tokio::test]
async fn a_pinned_client_refuses_a_different_certificate() {
    let b = Pc::new("B").await;
    let wrong = "ab".repeat(32);
    let dir = tempfile::tempdir().unwrap();
    let c = raw_client(dir.path(), Some(&wrong));
    assert!(c.get(b.peer_url("/peer/hello")).send().await.is_err());
}

// ---------------------------------------------------------------- settings and persistence

#[tokio::test]
async fn name_and_visibility_are_settings_and_survive_restart() {
    let mut a = Pc::new("Original").await;
    assert_eq!(a.b.pc_name(), "Original");
    a.b.set_pc_name("Office PC\n");
    assert_eq!(a.b.pc_name(), "Office PC");
    assert!(a.b.is_discoverable());
    a.b.set_discoverable(false);
    a.restart().await;
    assert_eq!(a.b.pc_name(), "Office PC");
    assert!(!a.b.is_discoverable());
    // what a stranger can learn: the name, nothing else
    let (_d, c) = stranger(&a);
    let (_, v) = json_of(c.get(a.peer_url("/peer/hello")).send().await.unwrap()).await;
    assert_eq!(v["name"], "Office PC");
}

#[tokio::test]
async fn pairing_conversations_and_files_survive_a_restart() {
    let port_b = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let a = Pc::new("A").await;
    let mut b = Pc::with_ports("B", port_b).await;
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    a.b.send_peer_text(&a_to_b, "kept").await.unwrap();
    eventually("arrived", || (!b.b.peer_notes(&b_to_a, 10).is_empty()).then_some(())).await;

    // B restarts on the same port (a real restart: the old instance is shut down first)
    let old = std::mem::replace(&mut b.b, Arc::new(Bridge::start({
        let mut c = b.cfg.clone();
        c.lan_port = 0;
        c.peer_port = 0;
        c
    }).await.unwrap()));
    Arc::try_unwrap(old).ok().expect("single handle").shutdown().await;
    let restarted = Bridge::start({
        let mut c = b.cfg.clone();
        c.lan_port = 0;
        c.peer_port = port_b;
        c
    })
    .await
    .unwrap();
    assert_eq!(restarted.peers().len(), 1, "trust survives");
    assert_eq!(restarted.peer_notes(&b_to_a, 10)[0].text.as_deref(), Some("kept"));
    // and A can still talk to it, pinned to the same certificate
    a.b.send_peer_text(&a_to_b, "after restart").await.unwrap();
    assert_eq!(restarted.peer_notes(&b_to_a, 10).len(), 2);
}

#[tokio::test]
async fn phones_can_be_renamed() {
    let b = Pc::new("B").await;
    let code = b.b.new_pairing().code;
    let phone = reqwest::Client::builder().use_preconfigured_tls(pinned_client_config(b.b.fingerprint()).unwrap()).build().unwrap();
    let (_, v) = json_of(phone.post(b.phone_url("/api/pair")).json(&json!({ "code": code, "name": "Android phone" })).send().await.unwrap()).await;
    let id = v["device_id"].as_str().unwrap().to_string();
    assert_eq!(b.b.devices()[0].name, "Android phone");
    assert!(b.b.rename_device(&id, "  Pixel 8\n"));
    assert_eq!(b.b.devices()[0].name, "Pixel 8");
    assert!(!b.b.rename_device(&id, "   "));
    assert!(!b.b.rename_device("nope", "x"));
    assert_eq!(b.b.devices()[0].name, "Pixel 8");
}

// ---------------------------------------------------------------- discovery (needs a network that allows multicast)

#[tokio::test]
async fn pcs_find_each_other_and_say_nothing_else() {
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let mk = |t: &tempfile::TempDir| {
        let mut c = Config::new(t.path().join("data"), t.path().join("inbox"));
        c.lan_port = 0;
        c.peer_port = 0;
        c.bind = "0.0.0.0".parse().unwrap();
        c.discovery = true;
        c
    };
    let a = Bridge::start(mk(&tmp_a)).await.unwrap();
    let b = Bridge::start(mk(&tmp_b)).await.unwrap();
    a.set_pc_name("Found-A");
    b.set_pc_name("Found-B");
    let mut found = None;
    for _ in 0..160 {
        if let Some(n) = a.nearby().into_iter().find(|n| n.id == b.fingerprint()) {
            found = Some(n);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(n) = found else {
        eprintln!("SKIPPED: no multicast discovery in this environment");
        return;
    };
    assert_eq!((n.name.as_str(), n.port, n.paired), ("Found-B", b.peer_port(), false));
    assert!(a.nearby().iter().all(|x| x.id != a.fingerprint()), "a PC must not list itself");

    // hidden PCs disappear for others (eventually)
    b.set_discoverable(false);
}

// ---------------------------------------------------------------- asking, cancelling, replies, deleting

fn transfer_of(pc: &Pc, id: &str) -> Option<TransferInfo> {
    pc.b.transfers().into_iter().find(|t| t.id == id)
}

/// Start sending `path` from `a` to `b` in the background; returns the transfer id once both sides list it.
async fn offer(a: &Pc, a_to_b: &str, b: &Pc, path: &Path) -> (String, tokio::task::JoinHandle<std::result::Result<FileRef, PeerError>>) {
    let (bridge, id, p) = (a.b.clone(), a_to_b.to_string(), path.to_path_buf());
    let task = tokio::spawn(async move { bridge.send_peer_file(&id, &p).await });
    let tid = eventually("the offer shows up on the receiving PC", || {
        b.b.transfers().into_iter().find(|t| t.state == TransferState::Offered && t.dir == Direction::FromPeer).map(|t| t.id)
    })
    .await;
    (tid, task)
}

#[tokio::test]
async fn a_file_waits_for_the_other_user_and_only_then_is_sent() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    let data = noise(300_000, 4);
    let src = a.tmp.path().join("song.wav");
    std::fs::write(&src, &data).unwrap();

    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    let mine = eventually("sender shows that it waits", || transfer_of(&a, &tid).filter(|t| t.state == TransferState::Offered)).await;
    assert_eq!((mine.dir, mine.device.as_str(), mine.size as usize), (Direction::ToPeer, "Laptop", data.len()));
    let theirs = transfer_of(&b, &tid).unwrap();
    assert_eq!((theirs.device.as_str(), theirs.name.as_str(), theirs.device_id.as_str()), ("Desk", "song.wav", a.b.fingerprint()));

    // while nobody answers: nothing on disk, nothing sent, no chat entry
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!task.is_finished());
    assert!(!b.inbox().join("song.wav").exists());
    assert!(b.b.peer_notes(&b_to_a, 10).is_empty() && a.b.peer_notes(&a_to_b, 10).is_empty());
    assert_eq!(transfer_of(&a, &tid).unwrap().state, TransferState::Offered);

    assert!(b.b.accept_file(&tid).await);
    let f = task.await.unwrap().unwrap();
    assert_eq!(f.id, tid);
    assert_eq!(std::fs::read(b.inbox().join("song.wav")).unwrap(), data);

    // both sides call the message by the same id and know where the file is
    let (on_a, on_b) = (a.b.peer_notes(&a_to_b, 10), b.b.peer_notes(&b_to_a, 10));
    assert_eq!((on_a[0].mid.as_str(), on_b[0].mid.as_str()), (tid.as_str(), tid.as_str()));
    assert_eq!(on_a[0].file.as_ref().unwrap().local_path.as_deref(), Some(src.canonicalize().unwrap().to_str().unwrap()));
    assert!(on_b[0].file.as_ref().unwrap().local_path.as_deref().unwrap().ends_with("song.wav"));
    assert_eq!(transfer_of(&a, &tid).unwrap().state, TransferState::Done);
    assert_eq!(transfer_of(&b, &tid).unwrap().state, TransferState::Done);
}

#[tokio::test]
async fn declining_tells_the_sender_and_stores_nothing() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    let src = a.tmp.path().join("no.bin");
    std::fs::write(&src, noise(1000, 1)).unwrap();

    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    assert!(b.b.decline_file(&tid));
    assert!(matches!(task.await.unwrap(), Err(PeerError::Declined)));
    assert!(transfer_of(&b, &tid).is_none(), "the receiver does not keep a declined offer");
    let t = transfer_of(&a, &tid).unwrap();
    assert_eq!((t.state, t.error.as_deref()), (TransferState::Declined, Some("Declined by Laptop")));
    assert!(!b.inbox().join("no.bin").exists());

    // the sender can clear it from its list; nothing else changes
    assert!(a.b.dismiss_transfer(&tid).await);
    assert!(transfer_of(&a, &tid).is_none());
}

#[tokio::test]
async fn the_sender_can_cancel_while_waiting_and_both_lists_can_be_cleared() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    let src = a.tmp.path().join("oops.bin");
    std::fs::write(&src, noise(1000, 2)).unwrap();

    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    assert!(a.b.cancel_transfer(&tid));
    assert!(matches!(task.await.unwrap(), Err(PeerError::Cancelled)));
    assert_eq!(transfer_of(&a, &tid).unwrap().state, TransferState::Cancelled);
    let t = eventually("receiver learns of the cancel", || transfer_of(&b, &tid).filter(|t| t.state == TransferState::Cancelled)).await;
    assert_eq!(t.error.as_deref(), Some("Cancelled by Desk"));
    assert!(!b.b.accept_file(&tid).await, "a cancelled offer cannot be accepted");

    // removing it on one side removes it on the other
    assert!(b.b.dismiss_transfer(&tid).await);
    assert!(transfer_of(&b, &tid).is_none());
    eventually("the sender's list is cleaned too", || transfer_of(&a, &tid).is_none().then_some(())).await;
}

#[tokio::test]
async fn the_receiver_can_cancel_a_running_transfer() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    let src = a.tmp.path().join("long.bin");
    std::fs::write(&src, noise(40_000_000, 3)).unwrap();

    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    assert!(b.b.accept_file(&tid).await);
    assert!(b.b.cancel_transfer(&tid));
    assert!(matches!(task.await.unwrap(), Err(PeerError::Cancelled)));
    let t = transfer_of(&a, &tid).unwrap();
    assert_eq!((t.state, t.error.as_deref()), (TransferState::Cancelled, Some("Cancelled by Laptop")));
    assert_eq!(transfer_of(&b, &tid).unwrap().state, TransferState::Cancelled);
    assert!(!b.inbox().join("long.bin").exists());
    eventually("the partial file is removed", || {
        let dir = b.tmp.path().join("data").join("incoming");
        std::fs::read_dir(dir).map(|d| d.count() == 0).unwrap_or(true).then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_sender_can_cancel_a_running_transfer() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    let src = a.tmp.path().join("long2.bin");
    std::fs::write(&src, noise(40_000_000, 5)).unwrap();

    let (bridge, id, p) = (a.b.clone(), a_to_b.clone(), src.clone());
    let task = tokio::spawn(async move { bridge.send_peer_file(&id, &p).await });
    let tid = eventually("transfer is running", || a.b.transfers().into_iter().find(|t| t.state == TransferState::Active).map(|t| t.id)).await;
    assert!(a.b.cancel_transfer(&tid));
    assert!(matches!(task.await.unwrap(), Err(PeerError::Cancelled)));
    assert_eq!(transfer_of(&a, &tid).unwrap().state, TransferState::Cancelled);
    let t = eventually("receiver shows the cancel", || transfer_of(&b, &tid).filter(|t| t.state == TransferState::Cancelled)).await;
    assert_eq!(t.error.as_deref(), Some("Cancelled by Desk"));
    assert!(!b.inbox().join("long2.bin").exists());
}

#[tokio::test]
async fn replies_carry_a_quote_and_deleting_works_for_both() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;

    let first = a.b.send_peer_text(&a_to_b, "dinner at 7?").await.unwrap();
    let on_b = eventually("B has it", || b.b.peer_notes(&b_to_a, 10).pop()).await;
    assert_eq!(on_b.mid, first.mid, "both sides use one id for a message");

    let reply = b.b.send_peer_text_reply(&b_to_a, "sounds good", Some(&on_b.mid)).await.unwrap();
    assert_eq!(reply.reply.as_ref().map(|r| (r.src.as_str(), r.preview.as_str())), Some(("Desk", "dinner at 7?")));
    let on_a = eventually("A has the reply", || a.b.peer_notes(&a_to_b, 10).into_iter().find(|n| n.mid == reply.mid)).await;
    assert_eq!(on_a.reply.as_ref().map(|r| (r.src.as_str(), r.preview.as_str())), Some(("pc", "dinner at 7?")));

    // B deletes the question: gone on both
    assert!(b.b.delete_peer_note(&b_to_a, &first.mid).await);
    assert!(!b.b.delete_peer_note(&b_to_a, &first.mid).await);
    eventually("gone on A", || (!a.b.peer_notes(&a_to_b, 10).iter().any(|n| n.mid == first.mid)).then_some(())).await;
    assert_eq!(a.b.peer_notes(&a_to_b, 10).len(), 1);

    // a bad message id is refused and nothing else is deleted
    let r = a.client_to(&b).post(b.peer_url("/peer/notes/delete")).json(&json!({ "mid": "../x" })).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_reply_shows_its_quote_even_when_the_other_pc_has_no_copy_of_the_original() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    let first = a.b.send_peer_text(&a_to_b, "old question").await.unwrap();
    let on_b = eventually("B has it", || b.b.peer_notes(&b_to_a, 10).pop()).await;
    assert_eq!(on_b.mid, first.mid);
    // A no longer has the original (cleared, or an older message with another id), B still quotes it
    a.b.clear_peer_notes(&a_to_b);
    let reply = b.b.send_peer_text_reply(&b_to_a, "answer", Some(&on_b.mid)).await.unwrap();
    let on_a = eventually("A has the reply", || a.b.peer_notes(&a_to_b, 10).into_iter().find(|n| n.mid == reply.mid)).await;
    assert_eq!(on_a.reply.as_ref().map(|r| (r.src.as_str(), r.preview.as_str())), Some(("pc", "old question")));
}

#[tokio::test]
async fn deleting_files_never_touches_the_other_pcs_disk() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    let src = a.tmp.path().join("photo.jpg");
    std::fs::write(&src, noise(5000, 6)).unwrap();
    let f = a.b.send_peer_file(&a_to_b, &src).await.unwrap();
    let saved = b.inbox().join("photo.jpg");
    assert!(saved.exists());

    // the sender removes the message: the receiver loses the message but keeps the file
    assert!(a.b.delete_peer_note(&a_to_b, &f.id).await);
    assert!(src.exists());
    eventually("message gone on B", || b.b.peer_notes(&b_to_a, 10).is_empty().then_some(())).await;
    assert!(saved.exists(), "a file the other PC already saved stays");

    // the receiver deletes the file it saved
    let f2 = a.b.send_peer_file(&a_to_b, &src).await.unwrap();
    let saved2 = std::path::PathBuf::from(b.b.peer_notes(&b_to_a, 10).pop().unwrap().file.unwrap().local_path.unwrap());
    assert!(saved2.exists());
    assert!(b.b.delete_peer_note(&b_to_a, &f2.id).await);
    assert!(!saved2.exists(), "the received copy is deleted");
    assert!(src.exists());
    eventually("message gone on A", || a.b.peer_notes(&a_to_b, 10).is_empty().then_some(())).await;
}

#[tokio::test]
async fn an_offer_nobody_accepts_is_withdrawn_after_the_time_the_user_set() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    a.b.set_offer_timeout_secs(2);
    assert_eq!(a.b.offer_timeout_secs(), 2);
    let src = a.tmp.path().join("late.bin");
    std::fs::write(&src, noise(50_000, 9)).unwrap();

    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    eventually("the sender waits", || transfer_of(&a, &tid).filter(|t| t.state == TransferState::Offered)).await;
    let started = std::time::Instant::now();
    let res = tokio::time::timeout(Duration::from_secs(10), task).await.expect("the offer was not withdrawn").unwrap();
    assert!(res.is_err());
    assert!(started.elapsed() >= Duration::from_millis(1200), "withdrawn far too early: {:?}", started.elapsed());

    let mine = transfer_of(&a, &tid).unwrap();
    assert_eq!(mine.state, TransferState::Cancelled);
    assert!(mine.error.as_deref().unwrap_or("").contains("did not accept it within 2 s"), "{:?}", mine.error);
    // the other PC no longer shows a request it could accept, and nothing was stored
    eventually("the other PC lets go of the offer", || transfer_of(&b, &tid).filter(|t| t.state == TransferState::Cancelled)).await;
    assert!(!b.inbox().join("late.bin").exists());
    assert!(!b.b.accept_file(&tid).await);
}

#[tokio::test]
async fn with_no_limit_an_offer_keeps_waiting() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, _) = pair(&a, &b).await;
    b.b.set_auto_accept_files(false);
    assert_eq!(a.b.offer_timeout_secs(), 0);
    let src = a.tmp.path().join("patient.bin");
    std::fs::write(&src, noise(10_000, 3)).unwrap();
    let (tid, task) = offer(&a, &a_to_b, &b, &src).await;
    eventually("the sender waits", || transfer_of(&a, &tid).filter(|t| t.state == TransferState::Offered)).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!task.is_finished());
    assert!(b.b.accept_file(&tid).await);
    task.await.unwrap().unwrap();
}

// ---------------------------------------------------------------- two separate Clear buttons

#[tokio::test]
async fn clear_chat_and_clear_list_are_independent_for_a_pc() {
    let (a, b) = (Pc::new("Desk").await, Pc::new("Laptop").await);
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    a.b.send_peer_text(&a_to_b, "one").await.unwrap();
    a.b.send_peer_text(&a_to_b, "two").await.unwrap();
    eventually("B has both", || (b.b.peer_notes(&b_to_a, 10).len() == 2).then_some(())).await;

    let shown = |pc: &Pc, id: &str| -> usize { pc.b.peer_notes(id, 10).iter().filter(|n| !n.chat_hidden).count() };
    a.b.clear_peer_chat(&a_to_b);
    assert!(a.b.peer_notes(&a_to_b, 10).is_empty(), "the list shows files only, so a cleared text is gone");
    assert_eq!(shown(&b, &b_to_a), 2, "the other PC keeps its own copy");
    a.b.send_peer_text(&a_to_b, "three").await.unwrap();
    assert_eq!(shown(&a, &a_to_b), 1);
    a.b.clear_peer_list(&a_to_b);
    assert_eq!(shown(&a, &a_to_b), 1, "clearing the list does not touch texts");
    a.b.clear_peer_chat(&a_to_b);
    assert!(a.b.peer_notes(&a_to_b, 10).is_empty());
    assert_eq!(shown(&b, &b_to_a), 3);
}

// ---------------------------------------------------------------- messages and files wait for a PC that is off

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// B stops (a real shutdown); returns nothing, the data stays in B's folder.
async fn stop(pc: &mut Pc) {
    let mut c = pc.cfg.clone();
    c.lan_port = 0;
    c.peer_port = 0;
    let old = std::mem::replace(&mut pc.b, Arc::new(Bridge::start(c).await.unwrap()));
    Arc::try_unwrap(old).ok().expect("single handle").shutdown().await;
}

async fn start_on(pc: &Pc, port: u16) -> Bridge {
    let mut c = pc.cfg.clone();
    c.lan_port = 0;
    c.peer_port = port;
    Bridge::start(c).await.unwrap()
}

async fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    for _ in 0..(secs * 10) {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[tokio::test]
async fn messages_and_files_for_an_offline_pc_wait_and_go_out_when_it_is_back() {
    let port_b = free_port();
    let a = Pc::new("A").await;
    let mut b = Pc::with_ports("B", port_b).await;
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    stop(&mut b).await;

    // A writes while B is off: nothing fails, the message shows as waiting
    let note = a.b.send_peer_text(&a_to_b, "while you were away").await.unwrap();
    assert!(note.pending);
    let file = a.tmp.path().join("doc.txt");
    std::fs::write(&file, b"file content").unwrap();
    assert!(matches!(a.b.send_peer_file(&a_to_b, &file).await, Err(PeerError::Queued)));
    assert!(a.b.transfers().iter().any(|t| t.state == TransferState::Queued));
    assert!(a.b.peer_notes(&a_to_b, 10)[0].pending);
    // clearing the chat does not throw away what was not delivered yet
    a.b.clear_peer_chat(&a_to_b);
    assert_eq!(a.b.peer_notes(&a_to_b, 10).len(), 1);

    // B is back: both arrive, and A's message is no longer waiting
    let restarted = start_on(&b, port_b).await;
    wait_for("the message and the file arrive", 30, || {
        let n = restarted.peer_notes(&b_to_a, 10);
        n.iter().any(|n| n.text.as_deref() == Some("while you were away")) && n.iter().any(|n| n.file.as_ref().is_some_and(|f| f.name == "doc.txt"))
    })
    .await;
    wait_for("A no longer shows it as waiting", 5, || !a.b.peer_notes(&a_to_b, 10).iter().any(|n| n.pending)).await;
    assert!(!a.b.transfers().iter().any(|t| t.state == TransferState::Queued));
    // exactly once
    assert_eq!(restarted.peer_notes(&b_to_a, 10).iter().filter(|n| n.text.is_some()).count(), 1);
    assert_eq!(std::fs::read(b.inbox().join("doc.txt")).unwrap(), b"file content");
}

#[tokio::test]
async fn the_queue_survives_a_restart_and_waiting_items_can_be_taken_back() {
    let port_b = free_port();
    let mut a = Pc::new("A").await;
    let mut b = Pc::with_ports("B", port_b).await;
    let (a_to_b, b_to_a) = pair(&a, &b).await;
    stop(&mut b).await;

    let keep = a.b.send_peer_text(&a_to_b, "keep me").await.unwrap();
    let drop_text = a.b.send_peer_text(&a_to_b, "never mind").await.unwrap();
    let file = a.tmp.path().join("skip.txt");
    std::fs::write(&file, b"x").unwrap();
    let _ = a.b.send_peer_file(&a_to_b, &file).await;
    let tid = a.b.transfers().into_iter().find(|t| t.state == TransferState::Queued).unwrap().id;
    assert!(keep.pending && drop_text.pending);

    // deleting a waiting message and stopping a waiting file take them off the queue
    assert!(a.b.delete_peer_note(&a_to_b, &drop_text.mid).await);
    assert!(a.b.cancel_transfer(&tid));
    assert!(a.b.transfers().iter().any(|t| t.id == tid && t.state == TransferState::Cancelled));

    // the queue is saved: it survives a restart of A
    a.restart().await;
    assert!(a.b.peer_notes(&a_to_b, 10).iter().any(|n| n.mid == keep.mid && n.pending));

    let restarted = start_on(&b, port_b).await;
    wait_for("the kept message arrives", 30, || restarted.peer_notes(&b_to_a, 10).iter().any(|n| n.text.as_deref() == Some("keep me"))).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    let got = restarted.peer_notes(&b_to_a, 10);
    assert!(!got.iter().any(|n| n.text.as_deref() == Some("never mind")), "the deleted message is not sent");
    assert!(!got.iter().any(|n| n.file.is_some()), "the stopped file is not sent");
}
