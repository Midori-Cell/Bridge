//! PC <-> PC: pairing, trust, conversations, and the routes other PCs talk to.
//!
//! Privacy and isolation, by construction:
//! * Other PCs connect to their own port (mutual TLS) and only reach the `/peer/*` routes below.
//!   The phone routes (`/api/*`) are not served there, so a PC can never see the phone's notes, files or pairing.
//! * Phones connect to the phone port, which has no `/peer/*` routes.
//! * A PC is identified by the SHA-256 fingerprint of its certificate. Anything that is not on the
//!   paired list is refused, except the pairing handshake.
//! * What a PC announces on the network is its name and fingerprint. Nothing else.
//!
//! Pairing (like Bluetooth "numeric comparison", with a commitment so it cannot be ground down):
//!
//! 1. A -> B: `commit = SHA256(na)`.
//! 2. B -> A: `nb`.
//! 3. A -> B: reveals `na`; B checks the commitment.
//! 4. Both show `code = H(fp_A, fp_B, na, nb)` as six digits; each user compares the two screens and confirms.
//! 5. B's user confirms first (B then only waits), A's user confirms, A tells B (`complete`), and only then do
//!    both PCs save each other. Neither PC shows the other as paired before both users confirmed.
//!
//! A man in the middle has two different certificates, so the two screens show different codes, and
//! because A committed to `na` before seeing `nb`, he cannot steer them to match.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRequestParts, Path as UrlPath, Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::watch;

use crate::error::ApiError;
use crate::model::*;
use crate::server::JsonBody;
use crate::state::{clean_name, delete_saved_file, reply_for, Shared, MAX_TEXT_CHARS};
use crate::transfer::hash_file;
use crate::util::{now_ts, random_hex, sanitize_name};

const PAIR_TTL: Duration = Duration::from_secs(120);
const MAX_PENDING_PAIRS: usize = 4;
const PAIR_RATE: (usize, Duration) = (10, Duration::from_secs(60));
const MAX_WAIT_SECS: f64 = 25.0;
const PEER_HISTORY: usize = 200;
const UPLOAD_CHUNK: usize = 4 * 1024 * 1024;
const MAX_RETRIES: u32 = 6;
const ONLINE_WINDOW: Duration = Duration::from_secs(60);
const PROTOCOL: u32 = 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Resolves when `true` is sent on the channel; never if the sender is dropped without sending.
async fn cancelled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

// ------------------------------------------------------------------------------------ state

/// Fingerprint of the client certificate on this connection (set per connection by the accept loop).
#[derive(Clone, Debug)]
pub(crate) struct PeerFp(pub Option<String>);

/// Source address of the connection a request came in on.
#[derive(Clone, Copy)]
pub(crate) struct PeerIp(pub IpAddr);

#[derive(Clone, Copy, PartialEq, Eq)]
enum PIn {
    AwaitingReveal,
    Pending,
    /// The user of this PC confirmed the code; waiting for the asking PC's user.
    Accepted,
    /// Both confirmed; the asking PC is saved.
    Done,
    Declined,
}

struct PendingIn {
    fp: String,
    name: String,
    commit: [u8; 32],
    nb: [u8; 16],
    code: String,
    state: PIn,
    created: Instant,
    /// Where to reach the asking PC again (its source address plus the port it says it listens on).
    addr: Option<String>,
}

struct OutRec {
    host: IpAddr,
    port: u16,
    peer_fp: String,
    peer_name: String,
    code: String,
    state: OutPairState,
    error: Option<String>,
    user_confirmed: bool,
    peer_accepted: bool,
}

pub(crate) struct NearbyRec {
    pub name: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
}

pub(crate) struct PeerState {
    pub nearby: Mutex<HashMap<String, NearbyRec>>,
    incoming: Mutex<HashMap<String, PendingIn>>,
    outgoing: Mutex<HashMap<String, OutRec>>,
    wake: watch::Sender<u64>,
    rate: Mutex<VecDeque<Instant>>,
    last_ok: Mutex<HashMap<String, Instant>>,
    /// Files this PC is sending, by transfer id: send `true` to cancel.
    sending: Mutex<HashMap<String, watch::Sender<bool>>>,
    pub discovery: Mutex<Option<crate::discovery::Discovery>>,
}

impl Default for PeerState {
    fn default() -> Self {
        Self {
            nearby: Mutex::new(HashMap::new()),
            incoming: Mutex::new(HashMap::new()),
            outgoing: Mutex::new(HashMap::new()),
            wake: watch::channel(0).0,
            rate: Mutex::new(VecDeque::new()),
            last_ok: Mutex::new(HashMap::new()),
            sending: Mutex::new(HashMap::new()),
            discovery: Mutex::new(None),
        }
    }
}

/// What the user can do about a failed contact with another PC.
#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    #[error("That PC is not paired.")]
    NotPaired,
    #[error("The other PC could not be reached. Is it on and running Bridge?")]
    Unreachable,
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Other(String),
    #[error("Cancelled.")]
    Cancelled,
    #[error("The other PC declined the file.")]
    Declined,
}

fn explain(code: &str) -> String {
    match code {
        "not_paired" => "The other PC no longer has this PC paired. Pair them again.".into(),
        "rate_limited" | "too_many_requests" => "The other PC is busy with pairing requests. Try again in a minute.".into(),
        "hash_mismatch" => "The file arrived damaged. Try again.".into(),
        "disk_full" => "The other PC's disk is full.".into(),
        "file_too_large" => "The file is too large for the other PC.".into(),
        "too_many_transfers" => "Too many transfers at once.".into(),
        "text_too_long" => "That text is too long.".into(),
        "that_is_me" => "That is this PC.".into(),
        other => format!("The other PC refused ({other})."),
    }
}

// ------------------------------------------------------------------------------------ helpers

pub(crate) fn pair_code(init_fp: &str, resp_fp: &str, na: &[u8], nb: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"bridge-pair-v1|");
    h.update(init_fp.as_bytes());
    h.update(b"|");
    h.update(resp_fp.as_bytes());
    h.update(b"|");
    h.update(na);
    h.update(nb);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

fn commitment(na: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"bridge-commit-v1|");
    h.update(na);
    h.finalize().into()
}

fn random_bytes16() -> [u8; 16] {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

fn is_fp(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn host_port(addr: IpAddr, port: u16) -> String {
    match addr {
        IpAddr::V6(a) => format!("[{a}]:{port}"),
        IpAddr::V4(a) => format!("{a}:{port}"),
    }
}

// ------------------------------------------------------------------------------------ state: trust, names, notes

impl Shared {
    pub(crate) fn pc_name(&self) -> String {
        let stored = lock(&self.persist).pc_name.clone().filter(|n| !n.trim().is_empty());
        stored.unwrap_or_else(|| {
            let h = gethostname::gethostname().to_string_lossy().into_owned();
            let h = clean_name(&h);
            if h.is_empty() { "This PC".to_string() } else { h }
        })
    }

    pub(crate) fn set_pc_name(&self, name: &str) {
        let name = clean_name(name);
        {
            let mut p = lock(&self.persist);
            p.pc_name = if name.is_empty() { None } else { Some(name) };
            self.save(&p);
        }
        self.reannounce();
    }

    pub(crate) fn is_hidden(&self) -> bool {
        lock(&self.persist).hidden
    }

    pub(crate) fn set_hidden(&self, hidden: bool) {
        {
            let mut p = lock(&self.persist);
            p.hidden = hidden;
            self.save(&p);
        }
        self.reannounce();
    }

    fn reannounce(&self) {
        if let Some(d) = lock(&self.peer.discovery).as_ref() {
            d.refresh(&self.pc_name(), &self.fingerprint, self.peer_port, self.is_hidden());
        }
    }

    pub(crate) fn peer_exists(&self, fp: &str) -> bool {
        lock(&self.persist).peers.contains_key(fp)
    }

    /// The trusted PC behind a certificate fingerprint, as a `Device` (its id is the fingerprint).
    pub(crate) fn peer_device(&self, fp: &str) -> Option<Device> {
        let p = lock(&self.persist);
        let rec = p.peers.get(fp)?;
        Some(Device { id: fp.to_string(), name: rec.name.clone(), created: rec.created, last_seen: None })
    }

    fn touch_peer(&self, fp: &str) {
        lock(&self.peer.last_ok).insert(fp.to_string(), Instant::now());
    }

    pub(crate) fn peers(&self) -> Vec<Peer> {
        let recs: Vec<(String, String, f64)> =
            lock(&self.persist).peers.iter().map(|(id, r)| (id.clone(), r.name.clone(), r.created)).collect();
        let nearby = lock(&self.peer.nearby);
        let ok = lock(&self.peer.last_ok);
        recs.into_iter()
            .map(|(id, name, created)| {
                let online = nearby.contains_key(&id) || ok.get(&id).is_some_and(|t| t.elapsed() < ONLINE_WINDOW);
                Peer { id, name, created, online }
            })
            .collect()
    }

    pub(crate) fn nearby(&self) -> Vec<Nearby> {
        let paired: Vec<String> = lock(&self.persist).peers.keys().cloned().collect();
        let n = lock(&self.peer.nearby);
        let mut out: Vec<Nearby> = n
            .iter()
            .map(|(id, r)| Nearby { id: id.clone(), name: r.name.clone(), addrs: r.addrs.clone(), port: r.port, paired: paired.contains(id) })
            .collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }

    pub(crate) fn upsert_nearby(&self, fp: &str, name: &str, addrs: Vec<IpAddr>, port: u16) {
        if fp == self.fingerprint || !is_fp(fp) || addrs.is_empty() {
            return;
        }
        let name = clean_name(name);
        let name = if name.is_empty() { "Unnamed PC".to_string() } else { name };
        let changed = {
            let mut n = lock(&self.peer.nearby);
            let old = n.insert(fp.to_string(), NearbyRec { name, addrs: addrs.clone(), port });
            old.is_none_or(|o| o.port != port || o.addrs != addrs)
        };
        if changed {
            self.emit(Event::PeersChanged);
        }
    }

    pub(crate) fn remove_nearby_instance(&self, instance: &str) {
        let removed = {
            let mut n = lock(&self.peer.nearby);
            let before = n.len();
            n.retain(|fp, _| !fp.starts_with(instance));
            n.len() != before
        };
        if removed {
            self.emit(Event::PeersChanged);
        }
    }

    fn update_peer_name(&self, fp: &str, name: &str) {
        let name = clean_name(name);
        if name.is_empty() {
            return;
        }
        let changed = {
            let mut p = lock(&self.persist);
            match p.peers.get_mut(fp) {
                Some(r) if r.name != name => {
                    r.name = name;
                    self.save(&p);
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit(Event::PeersChanged);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_peer_note(
        &self,
        fp: &str,
        src: &str,
        kind: NoteKind,
        mid: String,
        text: Option<String>,
        file: Option<FileRef>,
        reply: Option<Reply>,
    ) -> Note {
        let note = {
            let mut p = lock(&self.persist);
            let id = p.next_id;
            p.next_id += 1;
            let list = p.peer_notes.entry(fp.to_string()).or_default();
            let note = Note { id, mid, ts: now_ts(), src: src.to_string(), kind, text, file, reply };
            list.push(note.clone());
            if list.len() > PEER_HISTORY {
                let n = list.len() - PEER_HISTORY;
                list.drain(..n);
            }
            self.save(&p);
            note
        };
        self.emit(Event::PeerNote { peer_id: fp.to_string(), note: note.clone() });
        note
    }

    /// Remove one message from a conversation. `delete_files`: also delete a received file from the
    /// inbox (the user pressed delete here); when the other PC asked for it, only the message goes.
    pub(crate) fn remove_peer_note(&self, fp: &str, mid: &str, delete_files: bool) -> bool {
        let removed = {
            let mut p = lock(&self.persist);
            let Some(list) = p.peer_notes.get_mut(fp) else { return false };
            let Some(pos) = list.iter().position(|n| n.mid == mid) else { return false };
            let note = list.remove(pos);
            if delete_files {
                if let Some(f) = note.file.as_ref().filter(|f| f.dir == Direction::FromPeer) {
                    delete_saved_file(&self.inbox(), f);
                }
            }
            self.save(&p);
            note
        };
        self.emit(Event::NoteRemoved { peer_id: Some(fp.to_string()), mid: removed.mid });
        true
    }

    pub(crate) fn has_peer_note(&self, fp: &str, mid: &str) -> bool {
        lock(&self.persist).peer_notes.get(fp).is_some_and(|l| l.iter().any(|n| n.mid == mid))
    }

    pub(crate) fn peer_notes(&self, fp: &str, limit: usize) -> Vec<Note> {
        let p = lock(&self.persist);
        p.peer_notes.get(fp).map(|l| l.iter().rev().take(limit).rev().cloned().collect()).unwrap_or_default()
    }

    pub(crate) fn clear_peer_notes(&self, fp: &str) {
        let mut p = lock(&self.persist);
        p.peer_notes.remove(fp);
        self.save(&p);
    }

    /// Remove a paired PC, its conversation and its half-received files.
    pub(crate) fn remove_peer(&self, fp: &str) -> bool {
        let existed = {
            let mut p = lock(&self.persist);
            let existed = p.peers.remove(fp).is_some();
            p.peer_notes.remove(fp);
            if existed {
                self.save(&p);
            }
            existed
        };
        if !existed {
            return false;
        }
        lock(&self.peer.last_ok).remove(fp);
        let gone: Vec<_> = {
            let mut inc = lock(&self.incoming);
            let ids: Vec<String> = inc.iter().filter(|(_, i)| i.device_id == fp).map(|(k, _)| k.clone()).collect();
            ids.iter().filter_map(|k| inc.remove(k)).collect()
        };
        for i in gone {
            crate::transfer::remove_incoming_files(&self.cfg, &i.id);
        }
        self.emit(Event::PeersChanged);
        true
    }

    fn wake_pairs(&self) {
        self.peer.wake.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn purge_pairs(&self) {
        lock(&self.peer.incoming).retain(|_, p| p.created.elapsed() < PAIR_TTL);
    }
}

// ------------------------------------------------------------------------------------ pairing: this PC answers

impl Shared {
    pub(crate) fn pair_request(&self, fp: &str, name: &str, commit_hex: &str, addr: Option<String>) -> Result<(String, String), ApiError> {
        if fp == self.fingerprint {
            return Err(ApiError::bad_request("that_is_me"));
        }
        let commit: [u8; 32] = hex::decode(commit_hex)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| ApiError::bad_request("bad_commit"))?;
        {
            let mut r = lock(&self.peer.rate);
            while r.front().is_some_and(|t| t.elapsed() > PAIR_RATE.1) {
                r.pop_front();
            }
            if r.len() >= PAIR_RATE.0 {
                return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
            }
            r.push_back(Instant::now());
        }
        self.purge_pairs();
        let name = clean_name(name);
        let name = if name.is_empty() { "Unnamed PC".to_string() } else { name };
        let id = random_hex(8);
        let nb = random_bytes16();
        {
            let mut inc = lock(&self.peer.incoming);
            inc.retain(|_, p| p.fp != fp); // one open request per PC
            if inc.len() >= MAX_PENDING_PAIRS {
                return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests"));
            }
            inc.insert(
                id.clone(),
                PendingIn { fp: fp.to_string(), name, commit, nb, code: String::new(), state: PIn::AwaitingReveal, created: Instant::now(), addr },
            );
        }
        Ok((id, hex::encode(nb)))
    }

    pub(crate) fn pair_reveal(&self, fp: &str, id: &str, na_hex: &str) -> Result<(), ApiError> {
        self.purge_pairs();
        let na: [u8; 16] = hex::decode(na_hex).ok().and_then(|v| v.try_into().ok()).ok_or_else(|| ApiError::bad_request("bad_nonce"))?;
        let ev = {
            let mut inc = lock(&self.peer.incoming);
            let p = inc.get_mut(id).filter(|p| p.fp == fp).ok_or_else(ApiError::not_found)?;
            if p.state != PIn::AwaitingReveal {
                return Err(ApiError::conflict("already_revealed"));
            }
            if !bool::from(commitment(&na).as_slice().ct_eq(&p.commit)) {
                inc.remove(id);
                return Err(ApiError::new(StatusCode::FORBIDDEN, "bad_commit"));
            }
            p.code = pair_code(fp, &self.fingerprint, &na, &p.nb);
            p.state = PIn::Pending;
            IncomingPair { id: id.to_string(), name: p.name.clone(), code: p.code.clone() }
        };
        self.emit(Event::PairRequest(ev));
        self.wake_pairs();
        Ok(())
    }

    fn pair_state_name(s: PIn) -> &'static str {
        match s {
            PIn::AwaitingReveal => "awaiting_reveal",
            PIn::Pending => "pending",
            PIn::Accepted | PIn::Done => "accepted",
            PIn::Declined => "declined",
        }
    }

    pub(crate) fn pair_status(&self, fp: &str, id: &str) -> Option<&'static str> {
        self.purge_pairs();
        lock(&self.peer.incoming).get(id).filter(|p| p.fp == fp).map(|p| Self::pair_state_name(p.state))
    }

    /// The asking PC withdraws. If this PC had already accepted, the trust it granted is withdrawn too.
    pub(crate) fn pair_cancel(&self, fp: &str, id: &str) {
        let (removed, was_accepted) = {
            let mut inc = lock(&self.peer.incoming);
            match inc.get(id) {
                Some(p) if p.fp == fp => {
                    let acc = p.state == PIn::Done;
                    inc.remove(id);
                    (true, acc)
                }
                _ => (false, false),
            }
        };
        if was_accepted {
            self.remove_peer(fp); // emits PeersChanged
        }
        if removed {
            self.emit(Event::PeersChanged);
            self.wake_pairs();
        }
    }

    /// The user of this PC compared the codes and accepts. The asking PC is saved only once its user confirmed too.
    pub(crate) fn accept_pair(&self, id: &str) -> bool {
        self.purge_pairs();
        {
            let mut inc = lock(&self.peer.incoming);
            match inc.get_mut(id) {
                Some(p) if p.state == PIn::Pending => {
                    p.state = PIn::Accepted;
                    p.created = Instant::now(); // the asking user gets the full time to confirm
                }
                _ => return false,
            }
        }
        self.wake_pairs();
        true
    }

    /// The asking PC's user confirmed as well: now both sides save each other.
    pub(crate) fn pair_complete(&self, fp: &str, id: &str) -> Result<(), ApiError> {
        self.purge_pairs();
        let (name, addr) = {
            let mut inc = lock(&self.peer.incoming);
            let p = inc.get_mut(id).filter(|p| p.fp == fp).ok_or_else(ApiError::not_found)?;
            match p.state {
                PIn::Done => return Ok(()), // a repeated call after a lost answer
                PIn::Accepted => {}
                _ => return Err(ApiError::conflict("not_accepted")),
            }
            p.state = PIn::Done;
            (p.name.clone(), p.addr.clone())
        };
        {
            let mut p = lock(&self.persist);
            let created = p.peers.get(fp).map_or_else(now_ts, |r| r.created);
            let last_addr = addr.or_else(|| p.peers.get(fp).and_then(|r| r.last_addr.clone()));
            p.peers.insert(fp.to_string(), PeerRec { name, created, last_addr });
            self.save(&p);
        }
        self.touch_peer(fp);
        self.emit(Event::PeersChanged);
        self.wake_pairs();
        Ok(())
    }

    pub(crate) fn decline_pair(&self, id: &str) -> bool {
        let ok = {
            let mut inc = lock(&self.peer.incoming);
            match inc.get_mut(id) {
                Some(p) if matches!(p.state, PIn::Pending | PIn::AwaitingReveal | PIn::Accepted) => {
                    p.state = PIn::Declined;
                    true
                }
                _ => false,
            }
        };
        if ok {
            self.wake_pairs();
        }
        ok
    }

    pub(crate) fn pair_requests(&self) -> Vec<IncomingPair> {
        self.pairs_in_state(PIn::Pending)
    }

    /// Requests this PC's user accepted that wait for the other PC's user.
    pub(crate) fn pair_waiting(&self) -> Vec<IncomingPair> {
        self.pairs_in_state(PIn::Accepted)
    }

    fn pairs_in_state(&self, state: PIn) -> Vec<IncomingPair> {
        self.purge_pairs();
        lock(&self.peer.incoming)
            .iter()
            .filter(|(_, p)| p.state == state)
            .map(|(id, p)| IncomingPair { id: id.clone(), name: p.name.clone(), code: p.code.clone() })
            .collect()
    }
}

// ------------------------------------------------------------------------------------ client: talking to other PCs

type Observed = Arc<Mutex<Option<String>>>;

impl Shared {
    fn http(&self, pin: Option<&str>) -> Result<(reqwest::Client, Observed), PeerError> {
        let observed: Observed = Arc::new(Mutex::new(None));
        let cfg = crate::identity::peer_client_config(&self.identity, pin, observed.clone()).map_err(|e| PeerError::Other(e.to_string()))?;
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(cfg)
            .connect_timeout(Duration::from_secs(3))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| PeerError::Other(e.to_string()))?;
        Ok((client, observed))
    }

    fn peer_bases(&self, fp: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(n) = lock(&self.peer.nearby).get(fp) {
            let mut addrs = n.addrs.clone();
            addrs.sort_by_key(|a| a.is_ipv6()); // IPv4 first
            out.extend(addrs.into_iter().map(|a| format!("https://{}", host_port(a, n.port))));
        }
        if let Some(last) = lock(&self.persist).peers.get(fp).and_then(|r| r.last_addr.clone()) {
            let url = format!("https://{last}");
            if !out.contains(&url) {
                out.push(url);
            }
        }
        out
    }

    fn describe(e: &reqwest::Error) -> PeerError {
        let text = format!("{e:?}");
        if text.contains("does not match the paired PC") {
            PeerError::Other(
                "This PC's identity has changed since it was paired. It may be a different computer pretending to be it. Unpair it and pair again only if you trust the change."
                    .into(),
            )
        } else {
            PeerError::Unreachable
        }
    }

    /// Find a working address for a paired PC. Returns a client pinned to its certificate and the base URL.
    async fn connect(&self, fp: &str) -> Result<(reqwest::Client, String), PeerError> {
        if !self.peer_exists(fp) {
            return Err(PeerError::NotPaired);
        }
        let (client, _) = self.http(Some(fp))?;
        let mut last = PeerError::Unreachable;
        for base in self.peer_bases(fp) {
            match client.get(format!("{base}/peer/ping")).timeout(Duration::from_secs(4)).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body: Value = resp.json().await.unwrap_or(Value::Null);
                    if status == StatusCode::FORBIDDEN {
                        // The other PC unpaired us while this PC was off or away. Its answer is authentic (the
                        // connection is pinned to its certificate), so the pairing and chat are removed here too.
                        self.remove_peer(fp);
                        return Err(PeerError::Refused("The other PC no longer has this PC paired, so the pairing was removed here too.".into()));
                    }
                    if !status.is_success() {
                        continue;
                    }
                    if let Some(n) = body.get("name").and_then(Value::as_str) {
                        self.update_peer_name(fp, n);
                    }
                    self.touch_peer(fp);
                    let addr = base.trim_start_matches("https://").to_string();
                    {
                        let mut p = lock(&self.persist);
                        if let Some(r) = p.peers.get_mut(fp) {
                            if r.last_addr.as_deref() != Some(addr.as_str()) {
                                r.last_addr = Some(addr);
                                self.save(&p);
                            }
                        }
                    }
                    return Ok((client, base));
                }
                Err(e) => last = Self::describe(&e),
            }
        }
        Err(last)
    }

    async fn send_json(client: &reqwest::Client, url: String, body: Value, timeout: Duration) -> Result<(StatusCode, Value), PeerError> {
        let resp = client.post(url).json(&body).timeout(timeout).send().await.map_err(|e| Self::describe(&e))?;
        let status = resp.status();
        Ok((status, resp.json().await.unwrap_or(Value::Null)))
    }

    fn refused(status: StatusCode, body: &Value) -> PeerError {
        let code = body.get("error").and_then(Value::as_str).unwrap_or("error");
        let _ = status;
        PeerError::Refused(explain(code))
    }

    pub(crate) async fn send_peer_text(self: &Arc<Self>, fp: &str, text: &str, reply_to: Option<&str>) -> Result<Note, PeerError> {
        if text.trim().is_empty() {
            return Err(PeerError::Other("Nothing to send.".into()));
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(PeerError::Other(explain("text_too_long")));
        }
        let mid = random_hex(8);
        // The quote travels with the message (who wrote it and its first line), because the other PC
        // may not have the same copy of the original (older messages, or one that was cleared there).
        let reply = reply_to.and_then(|m| lock(&self.persist).peer_notes.get(fp).and_then(|list| reply_for(list, m)));
        let reply_json = reply.as_ref().map(|r| json!({ "mid": r.mid, "mine": r.src == "pc", "preview": r.preview }));
        let (client, base) = self.connect(fp).await?;
        let (status, body) = Self::send_json(
            &client,
            format!("{base}/peer/messages"),
            json!({ "text": text, "from": self.pc_name(), "mid": mid, "reply": reply_json }),
            Duration::from_secs(15),
        )
        .await?;
        if !status.is_success() {
            return Err(Self::refused(status, &body));
        }
        Ok(self.add_peer_note(fp, "pc", NoteKind::Text, mid, Some(text.to_string()), None, reply))
    }

    /// Delete a message here and ask the other PC to drop it from its view too (best effort: if it is
    /// off, its copy stays). A file it already saved stays on its disk.
    pub(crate) async fn delete_peer_note(self: &Arc<Self>, fp: &str, mid: &str) -> bool {
        if !self.remove_peer_note(fp, mid, true) {
            return false;
        }
        if let Ok((client, base)) = self.connect(fp).await {
            let _ = Self::send_json(&client, format!("{base}/peer/notes/delete"), json!({ "mid": mid }), Duration::from_secs(8)).await;
        }
        true
    }

    /// Take a transfer off the list here and on the other PC.
    pub(crate) async fn dismiss_peer_transfer(self: &Arc<Self>, fp: &str, id: &str) -> bool {
        let removed = self.dismiss_transfer(id, Some(fp));
        if removed {
            if let Ok((client, base)) = self.connect(fp).await {
                let _ = Self::send_json(&client, format!("{base}/peer/transfers/{id}/dismiss"), json!({}), Duration::from_secs(8)).await;
            }
        }
        removed
    }

    /// Stop sending a file (it may still be waiting to be accepted).
    pub(crate) fn cancel_sending(&self, id: &str) -> bool {
        match lock(&self.peer.sending).get(id) {
            Some(tx) => tx.send(true).is_ok(),
            None => false,
        }
    }

    /// Send a file to a paired PC: checksum first, then ask the other PC's user, then chunks that
    /// resume after a dropped connection. Either side can cancel at any time.
    pub(crate) async fn send_peer_file(self: &Arc<Self>, fp: &str, path: &Path) -> Result<FileRef, PeerError> {
        let dev = self.peer_device(fp).ok_or(PeerError::NotPaired)?;
        let path = tokio::fs::canonicalize(path).await.map_err(|e| PeerError::Other(format!("Cannot read the file: {e}")))?;
        let meta = tokio::fs::metadata(&path).await.map_err(|e| PeerError::Other(format!("Cannot read the file: {e}")))?;
        if !meta.is_file() {
            return Err(PeerError::Other("Only single files can be sent. Put folders in a zip first.".into()));
        }
        let name = sanitize_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let tid = random_hex(8);
        let info = TransferInfo {
            id: tid.clone(),
            name: name.clone(),
            size: meta.len(),
            done: 0,
            dir: Direction::ToPeer,
            state: TransferState::Hashing,
            device: dev.name.clone(),
            device_id: fp.to_string(),
            error: None,
        };
        self.set_transfer(info.clone());
        let (tx, rx) = watch::channel(false);
        lock(&self.peer.sending).insert(tid.clone(), tx);
        let result = self.send_peer_file_run(fp, &path, info.clone(), rx).await;
        lock(&self.peer.sending).remove(&tid);
        let mut info = info;
        match &result {
            Ok(_) => {}
            Err(PeerError::Cancelled) => {
                // the run may already have said who cancelled; keep that text
                let already = self.transfers().iter().any(|t| t.id == tid && t.state == TransferState::Cancelled);
                if !already {
                    info.state = TransferState::Cancelled;
                    self.set_transfer(info);
                }
            }
            Err(PeerError::Declined) => {
                info.state = TransferState::Declined;
                info.error = Some(format!("Declined by {}", dev.name));
                self.set_transfer(info);
            }
            Err(_) => {}
        }
        result
    }

    async fn send_peer_file_run(
        self: &Arc<Self>,
        fp: &str,
        path: &Path,
        mut info: TransferInfo,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<FileRef, PeerError> {
        let (tid, name, size) = (info.id.clone(), info.name.clone(), info.size);
        let peer_name = info.device.clone();
        let fail = |this: &Shared, mut info: TransferInfo, e: PeerError| {
            info.state = TransferState::Failed;
            info.error = Some(e.to_string());
            this.set_transfer(info);
            e
        };

        let p = path.to_path_buf();
        let hashing = tokio::task::spawn_blocking(move || hash_file(&p));
        let sha = tokio::select! {
            _ = cancelled(&mut cancel) => return Err(PeerError::Cancelled),
            h = hashing => match h {
                Ok(Ok(h)) => h,
                _ => return Err(fail(self, info, PeerError::Other("Cannot read the file.".into()))),
            },
        };
        let (client, base) = match self.connect(fp).await {
            Ok(v) => v,
            Err(e) => return Err(fail(self, info, e)),
        };
        let (status, body) = match Self::send_json(
            &client,
            format!("{base}/peer/files"),
            json!({ "id": tid, "name": name, "size": size, "sha256": sha }),
            Duration::from_secs(15),
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return Err(fail(self, info, e)),
        };
        if !status.is_success() {
            let e = Self::refused(status, &body);
            return Err(fail(self, info, e));
        }
        let remote_id = body.get("id").and_then(Value::as_str).unwrap_or(&tid).to_string();
        let mut state = body.get("state").and_then(Value::as_str).unwrap_or("receiving").to_string();

        let result = self.send_peer_file_body(&client, &base, &remote_id, path, &mut info, &mut state, &peer_name, &mut cancel).await;
        match result {
            Ok(()) => {}
            Err(PeerError::Cancelled) => {
                // tell the other PC (best effort), so it can show the cancellation and let its user remove it
                let _ = client.delete(format!("{base}/peer/files/{remote_id}")).timeout(Duration::from_secs(4)).send().await;
                return Err(PeerError::Cancelled);
            }
            Err(PeerError::Declined) => return Err(PeerError::Declined),
            Err(e) => {
                if matches!(e, PeerError::Other(ref m) if m.starts_with("Cancelled by")) {
                    info.state = TransferState::Cancelled;
                    info.error = Some(e.to_string());
                    self.set_transfer(info);
                    return Err(PeerError::Cancelled);
                }
                info.state = TransferState::Interrupted;
                info.error = Some(e.to_string());
                self.set_transfer(info);
                return Err(e);
            }
        }
        info.done = size;
        info.state = TransferState::Done;
        self.set_transfer(info);
        let fref = FileRef { id: tid.clone(), name, size, sha256: sha, dir: Direction::ToPeer, local_path: Some(path.to_string_lossy().into_owned()) };
        self.add_peer_note(fp, "pc", NoteKind::File, tid, None, Some(fref.clone()), None);
        Ok(fref)
    }

    /// Wait for the other user to accept, then upload. Errors `Cancelled`/`Declined` are final.
    #[allow(clippy::too_many_arguments)]
    async fn send_peer_file_body(
        &self,
        client: &reqwest::Client,
        base: &str,
        remote_id: &str,
        path: &Path,
        info: &mut TransferInfo,
        state: &mut String,
        peer_name: &str,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<(), PeerError> {
        let size = info.size;
        let status_url = format!("{base}/peer/files/{remote_id}");

        if state == "pending" {
            info.state = TransferState::Offered;
            self.set_transfer(info.clone());
            let mut fails = 0u32;
            loop {
                tokio::select! {
                    _ = cancelled(cancel) => return Err(PeerError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(700)) => {}
                }
                match client.get(&status_url).timeout(Duration::from_secs(8)).send().await {
                    Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
                        return Err(PeerError::Other(format!("Cancelled by {peer_name}")));
                    }
                    Ok(resp) => {
                        fails = 0;
                        let v: Value = resp.json().await.unwrap_or(Value::Null);
                        match v.get("state").and_then(Value::as_str).unwrap_or("") {
                            "pending" => {}
                            "declined" => return Err(PeerError::Declined),
                            "cancelled" => return Err(PeerError::Other(format!("Cancelled by {peer_name}"))),
                            _ => {
                                *state = "receiving".into();
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        fails += 1;
                        if fails > 20 {
                            return Err(Self::describe(&e));
                        }
                    }
                }
            }
        }

        info.state = TransferState::Active;
        self.set_transfer(info.clone());
        if state == "done" {
            return Ok(());
        }

        let mut offset = 0u64;
        let mut fails = 0u32;
        let mut file = tokio::fs::File::open(path).await.map_err(|e| PeerError::Other(format!("Cannot read the file: {e}")))?;
        let mut buf = vec![0u8; UPLOAD_CHUNK];
        while offset < size {
            file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| PeerError::Other(e.to_string()))?;
            let want = (size - offset).min(UPLOAD_CHUNK as u64) as usize;
            file.read_exact(&mut buf[..want]).await.map_err(|e| PeerError::Other(format!("Cannot read the file: {e}")))?;
            let url = format!("{status_url}?offset={offset}");
            let put = client.put(url).body(buf[..want].to_vec()).timeout(Duration::from_secs(120)).send();
            let sent = tokio::select! {
                _ = cancelled(cancel) => return Err(PeerError::Cancelled),
                r = put => r,
            };
            match sent {
                Ok(resp) if resp.status().is_success() => {
                    offset += want as u64;
                    fails = 0;
                    info.done = offset;
                    self.set_transfer(info.clone());
                }
                Ok(resp) => {
                    let status = resp.status();
                    let v: Value = resp.json().await.unwrap_or(Value::Null);
                    let code = v.get("error").and_then(Value::as_str).unwrap_or("");
                    if status == StatusCode::NOT_FOUND || code == "cancelled" {
                        return Err(PeerError::Other(format!("Cancelled by {peer_name}")));
                    }
                    if status == StatusCode::CONFLICT {
                        // the other side knows better how much it has
                        if let Some(r) = v.get("received").and_then(Value::as_u64) {
                            offset = r.min(size);
                            continue;
                        }
                    }
                    return Err(Self::refused(status, &v));
                }
                Err(e) => {
                    fails += 1;
                    if fails > MAX_RETRIES {
                        return Err(Self::describe(&e));
                    }
                    tokio::time::sleep(Duration::from_millis(500 * u64::from(fails))).await;
                    if let Ok(r) = client.get(&status_url).timeout(Duration::from_secs(10)).send().await {
                        if let Ok(v) = r.json::<Value>().await {
                            if let Some(rec) = v.get("received").and_then(Value::as_u64) {
                                offset = rec.min(size);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Tell a PC we no longer pair with it (best effort).
    pub(crate) async fn notify_unpair(self: &Arc<Self>, fp: &str, bases: Vec<String>) {
        let Ok((client, _)) = self.http(Some(fp)) else { return };
        for base in bases {
            if client.post(format!("{base}/peer/unpair")).timeout(Duration::from_secs(4)).send().await.is_ok() {
                return;
            }
        }
    }

    pub(crate) fn unpair_peer(self: &Arc<Self>, fp: &str) -> bool {
        let bases = self.peer_bases(fp);
        let existed = self.remove_peer(fp);
        if existed {
            let me = self.clone();
            let fp = fp.to_string();
            self.rt.spawn(async move { me.notify_unpair(&fp, bases).await });
        }
        existed
    }

    // -------------------------------------------------------------------- pairing: this PC asks

    fn out_snapshot(&self, id: &str) -> Option<OutgoingPair> {
        lock(&self.peer.outgoing).get(id).map(|o| OutgoingPair {
            id: id.to_string(),
            peer_name: o.peer_name.clone(),
            code: o.code.clone(),
            state: o.state,
            error: o.error.clone(),
        })
    }

    fn out_emit(&self, id: &str) {
        if let Some(s) = self.out_snapshot(id) {
            self.emit(Event::PairUpdate(s));
        }
    }

    /// Start pairing with the PC at `host:port`. Returns once the code is known; the rest happens in the background.
    pub(crate) async fn pair_start(self: &Arc<Self>, host: IpAddr, port: u16) -> Result<OutgoingPair, PeerError> {
        let na = random_bytes16();
        let commit = hex::encode(commitment(&na));
        let base = format!("https://{}", host_port(host, port));

        let (probe, observed) = self.http(None)?;
        let (status, body) = Self::send_json(
            &probe,
            format!("{base}/peer/pair"),
            json!({ "name": self.pc_name(), "commit": commit, "port": self.peer_port }),
            Duration::from_secs(10),
        )
        .await?;
        if !status.is_success() {
            return Err(Self::refused(status, &body));
        }
        let peer_fp = lock(&observed).clone().ok_or(PeerError::Unreachable)?;
        if peer_fp == self.fingerprint {
            return Err(PeerError::Other("That is this PC.".into()));
        }
        let id = body.get("id").and_then(Value::as_str).ok_or_else(|| PeerError::Other("Unexpected answer from the other PC.".into()))?.to_string();
        let nb: [u8; 16] = body
            .get("nonce")
            .and_then(Value::as_str)
            .and_then(|h| hex::decode(h).ok())
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| PeerError::Other("Unexpected answer from the other PC.".into()))?;
        let peer_name = body.get("name").and_then(Value::as_str).map(clean_name).filter(|n| !n.is_empty()).unwrap_or_else(|| "Unnamed PC".into());

        // From here on only the certificate we just saw is acceptable.
        let (client, _) = self.http(Some(&peer_fp))?;
        let (status, rb) = Self::send_json(&client, format!("{base}/peer/pair/{id}/reveal"), json!({ "nonce": hex::encode(na) }), Duration::from_secs(10)).await?;
        if !status.is_success() {
            return Err(Self::refused(status, &rb));
        }
        let code = pair_code(&self.fingerprint, &peer_fp, &na, &nb);
        lock(&self.peer.outgoing).insert(
            id.clone(),
            OutRec { host, port, peer_fp: peer_fp.clone(), peer_name, code, state: OutPairState::Waiting, error: None, user_confirmed: false, peer_accepted: false },
        );
        self.out_emit(&id);
        let me = self.clone();
        let task_id = id.clone();
        self.rt.spawn(async move { me.pair_poll(task_id, client, base).await });
        self.out_snapshot(&id).ok_or_else(|| PeerError::Other("Pairing was cancelled.".into()))
    }

    async fn pair_poll(self: Arc<Self>, id: String, client: reqwest::Client, base: String) {
        let started = Instant::now();
        loop {
            if !lock(&self.peer.outgoing).contains_key(&id) {
                return; // cancelled
            }
            if started.elapsed() > PAIR_TTL + Duration::from_secs(10) {
                self.out_finish(&id, OutPairState::Failed, Some("The other PC did not answer in time.".into()));
                return;
            }
            let r = client.get(format!("{base}/peer/pair/{id}?wait=20")).timeout(Duration::from_secs(30)).send().await;
            let Ok(resp) = r else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            if resp.status() == StatusCode::NOT_FOUND {
                self.out_finish(&id, OutPairState::Failed, Some("The other PC dropped the request.".into()));
                return;
            }
            let v: Value = resp.json().await.unwrap_or(Value::Null);
            match v.get("state").and_then(Value::as_str) {
                Some("accepted") => {
                    let confirmed = {
                        let mut o = lock(&self.peer.outgoing);
                        let Some(rec) = o.get_mut(&id) else { return };
                        rec.peer_accepted = true;
                        if !rec.user_confirmed {
                            rec.state = OutPairState::PeerAccepted;
                        }
                        rec.user_confirmed
                    };
                    if confirmed {
                        self.pair_finish(&id, &client, &base).await;
                        return;
                    }
                    self.out_emit(&id);
                    // The other PC keeps the request for PAIR_TTL after its user accepted. Wait for our user here;
                    // poll quickly so the confirmation feels instant and cancellation is noticed.
                    let accepted_at = Instant::now();
                    loop {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let st = lock(&self.peer.outgoing).get(&id).map(|r| (r.user_confirmed, r.state));
                        match st {
                            None | Some((_, OutPairState::Done)) => return,
                            Some((true, _)) => {
                                self.pair_finish(&id, &client, &base).await;
                                return;
                            }
                            Some(_) if accepted_at.elapsed() > PAIR_TTL - Duration::from_secs(5) => {
                                self.out_finish(&id, OutPairState::Failed, Some("The code was not confirmed in time.".into()));
                                return;
                            }
                            Some(_) => {}
                        }
                    }
                }
                Some("declined") => {
                    self.out_finish(&id, OutPairState::Declined, None);
                    return;
                }
                _ => {}
            }
        }
    }

    fn out_finish(&self, id: &str, state: OutPairState, error: Option<String>) {
        if let Some(o) = lock(&self.peer.outgoing).get_mut(id) {
            o.state = state;
            o.error = error;
        }
        self.out_emit(id);
    }

    /// Both users confirmed: tell the other PC, and when it has saved this PC, save it here too.
    async fn pair_finish(&self, id: &str, client: &reqwest::Client, base: &str) {
        let mut result = Err("The other PC did not answer.".to_string());
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            match Self::send_json(client, format!("{base}/peer/pair/{id}/complete"), json!({}), Duration::from_secs(10)).await {
                Ok((st, _)) if st.is_success() => {
                    result = Ok(());
                    break;
                }
                Ok((st, _)) if st == StatusCode::NOT_FOUND || st == StatusCode::CONFLICT => {
                    result = Err("The other PC cancelled the pairing.".to_string());
                    break;
                }
                _ => {}
            }
        }
        if let Err(e) = result {
            self.out_finish(id, OutPairState::Failed, Some(e));
            return;
        }
        let (fp, name, addr) = {
            let o = lock(&self.peer.outgoing);
            let Some(r) = o.get(id) else { return };
            (r.peer_fp.clone(), r.peer_name.clone(), host_port(r.host, r.port))
        };
        {
            let mut p = lock(&self.persist);
            let created = p.peers.get(&fp).map_or_else(now_ts, |r| r.created);
            p.peers.insert(fp.clone(), PeerRec { name, created, last_addr: Some(addr) });
            self.save(&p);
        }
        self.touch_peer(&fp);
        self.out_finish(id, OutPairState::Done, None);
        self.emit(Event::PeersChanged);
    }

    /// The user of this PC saw the same code on both screens. The background task finishes the pairing.
    pub(crate) fn pair_confirm(&self, id: &str) -> bool {
        let mut o = lock(&self.peer.outgoing);
        let Some(r) = o.get_mut(id) else { return false };
        if matches!(r.state, OutPairState::Done | OutPairState::Declined | OutPairState::Failed) {
            return false;
        }
        r.user_confirmed = true;
        true
    }

    pub(crate) fn pair_cancel_out(self: &Arc<Self>, id: &str) {
        let Some(rec) = lock(&self.peer.outgoing).remove(id) else { return };
        if matches!(rec.state, OutPairState::Done) {
            return;
        }
        let me = self.clone();
        let id = id.to_string();
        self.rt.spawn(async move {
            let Ok((client, _)) = me.http(Some(&rec.peer_fp)) else { return };
            let base = format!("https://{}", host_port(rec.host, rec.port));
            let _ = client.post(format!("{base}/peer/pair/{id}/cancel")).timeout(Duration::from_secs(4)).send().await;
        });
    }

    pub(crate) fn out_pairs(&self) -> Vec<OutgoingPair> {
        let ids: Vec<String> = lock(&self.peer.outgoing).keys().cloned().collect();
        ids.iter().filter_map(|i| self.out_snapshot(i)).collect()
    }
}

// ------------------------------------------------------------------------------------ routes other PCs call

pub(crate) struct AnyPeer(pub String);

impl FromRequestParts<Arc<Shared>> for AnyPeer {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _s: &Arc<Shared>) -> Result<Self, ApiError> {
        parts.extensions.get::<PeerFp>().and_then(|p| p.0.clone()).map(AnyPeer).ok_or_else(ApiError::unauthorized)
    }
}

pub(crate) struct TrustedPeer(pub Device);

impl FromRequestParts<Arc<Shared>> for TrustedPeer {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, s: &Arc<Shared>) -> Result<Self, ApiError> {
        let fp = parts.extensions.get::<PeerFp>().and_then(|p| p.0.clone()).ok_or_else(ApiError::unauthorized)?;
        let dev = s.peer_device(&fp).ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "not_paired"))?;
        s.touch_peer(&fp);
        Ok(TrustedPeer(dev))
    }
}

pub(crate) fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/peer/hello", get(hello))
        .route("/peer/pair", post(pair_request_h))
        .route("/peer/pair/{id}", get(pair_status_h))
        .route("/peer/pair/{id}/reveal", post(pair_reveal_h))
        .route("/peer/pair/{id}/complete", post(pair_complete_h))
        .route("/peer/pair/{id}/cancel", post(pair_cancel_h))
        .route("/peer/ping", get(ping))
        .route("/peer/messages", post(messages))
        .route("/peer/files", post(files_create))
        .route("/peer/files/{id}", get(files_get).put(files_put).delete(files_delete))
        .route("/peer/notes/delete", post(notes_delete))
        .route("/peer/transfers/{id}/dismiss", post(transfer_dismiss))
        .route("/peer/unpair", post(unpair_h))
        .fallback(|| async { ApiError::not_found() })
        .layer(middleware::map_response(crate::server::security_headers))
        .with_state(shared)
}

async fn hello(State(s): State<Arc<Shared>>, AnyPeer(fp): AnyPeer) -> Json<Value> {
    Json(json!({ "name": s.pc_name(), "protocol": PROTOCOL, "paired": s.peer_exists(&fp) }))
}

async fn ping(State(s): State<Arc<Shared>>, TrustedPeer(_dev): TrustedPeer) -> Json<Value> {
    Json(json!({ "ok": true, "name": s.pc_name(), "protocol": PROTOCOL }))
}

async fn pair_request_h(
    State(s): State<Arc<Shared>>,
    AnyPeer(fp): AnyPeer,
    axum::Extension(PeerIp(ip)): axum::Extension<PeerIp>,
    JsonBody(b): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let name = b.get("name").and_then(Value::as_str).unwrap_or("");
    let commit = b.get("commit").and_then(Value::as_str).unwrap_or("");
    let addr = b.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0).map(|p| host_port(ip, p));
    let (id, nonce) = s.pair_request(&fp, name, commit, addr)?;
    Ok(Json(json!({ "id": id, "nonce": nonce, "name": s.pc_name() })))
}

async fn pair_reveal_h(
    State(s): State<Arc<Shared>>,
    AnyPeer(fp): AnyPeer,
    UrlPath(id): UrlPath<String>,
    JsonBody(b): JsonBody,
) -> Result<Json<Value>, ApiError> {
    s.pair_reveal(&fp, &id, b.get("nonce").and_then(Value::as_str).unwrap_or(""))?;
    Ok(Json(json!({ "ok": true })))
}

async fn pair_complete_h(State(s): State<Arc<Shared>>, AnyPeer(fp): AnyPeer, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    s.pair_complete(&fp, &id)?;
    Ok(Json(json!({ "ok": true })))
}

async fn pair_status_h(
    State(s): State<Arc<Shared>>,
    AnyPeer(fp): AnyPeer,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let wait = q.get("wait").and_then(|v| v.parse::<f64>().ok()).filter(|w| w.is_finite()).unwrap_or(0.0).clamp(0.0, MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs_f64(wait);
    let mut rx = s.peer.wake.subscribe();
    loop {
        rx.borrow_and_update();
        let state = s.pair_status(&fp, &id).ok_or_else(ApiError::not_found)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if state == "accepted" || state == "declined" || remaining.is_zero() {
            return Ok(Json(json!({ "state": state })));
        }
        if !matches!(tokio::time::timeout(remaining, rx.changed()).await, Ok(Ok(()))) {
            let state = s.pair_status(&fp, &id).ok_or_else(ApiError::not_found)?;
            return Ok(Json(json!({ "state": state })));
        }
    }
}

async fn pair_cancel_h(State(s): State<Arc<Shared>>, AnyPeer(fp): AnyPeer, UrlPath(id): UrlPath<String>) -> Json<Value> {
    s.pair_cancel(&fp, &id);
    Json(json!({ "ok": true }))
}

async fn unpair_h(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer) -> Json<Value> {
    s.remove_peer(&dev.id);
    Json(json!({ "ok": true }))
}

fn valid_mid(m: &str) -> bool {
    !m.is_empty() && m.len() <= 32 && m.bytes().all(|b| b.is_ascii_alphanumeric())
}

async fn messages(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let text = b.get("text").and_then(Value::as_str).ok_or_else(|| ApiError::bad_request("empty_text"))?;
    if text.trim().is_empty() {
        return Err(ApiError::bad_request("empty_text"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError::bad_request("text_too_long"));
    }
    if let Some(from) = b.get("from").and_then(Value::as_str) {
        s.update_peer_name(&dev.id, from);
    }
    let mid = match b.get("mid").and_then(Value::as_str) {
        Some(m) if valid_mid(m) => m.to_string(),
        Some(_) => return Err(ApiError::bad_request("bad_id")),
        None => random_hex(8),
    };
    let name = s.peer_device(&dev.id).map_or(dev.name, |d| d.name);
    // The quote the other PC sent. "mine" is from its point of view, so it is the other way round here.
    let reply = match b.get("reply").and_then(Value::as_object) {
        Some(r) => {
            let mid = r.get("mid").and_then(Value::as_str).filter(|m| valid_mid(m));
            let preview: String = r.get("preview").and_then(Value::as_str).unwrap_or("").chars().take(160).collect();
            let mine = r.get("mine").and_then(Value::as_bool).unwrap_or(false);
            mid.map(|m| Reply { mid: m.to_string(), src: if mine { name.clone() } else { "pc".to_string() }, preview })
        }
        None => None,
    };
    if !s.has_peer_note(&dev.id, &mid) {
        s.add_peer_note(&dev.id, &name, NoteKind::Text, mid, Some(text.to_string()), None, reply);
    }
    Ok(Json(json!({ "ok": true, "name": s.pc_name() })))
}

/// The other PC deleted a message: it disappears from our view. A file we saved stays where it is.
async fn notes_delete(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let mid = b.get("mid").and_then(Value::as_str).filter(|m| valid_mid(m)).ok_or_else(|| ApiError::bad_request("bad_id"))?;
    s.remove_peer_note(&dev.id, mid, false);
    Ok(Json(json!({ "ok": true })))
}

async fn transfer_dismiss(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, UrlPath(id): UrlPath<String>) -> Json<Value> {
    s.dismiss_transfer(&id, Some(&dev.id));
    Json(json!({ "ok": true }))
}

async fn files_create(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, JsonBody(b): JsonBody) -> Result<Json<Value>, ApiError> {
    let name = b.get("name").and_then(Value::as_str).ok_or_else(|| ApiError::bad_request("bad_name"))?;
    let size = b.get("size").and_then(Value::as_u64).ok_or_else(|| ApiError::bad_request("bad_size"))?;
    let sha = match b.get("sha256") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.clone()),
        Some(_) => return Err(ApiError::bad_request("bad_sha256")),
    };
    let want_id = match b.get("id") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.clone()),
        Some(_) => return Err(ApiError::bad_request("bad_id")),
    };
    let out = s.create_incoming_for(&dev, true, name, size, sha, want_id).await?;
    Ok(Json(json!({ "id": out.id, "name": out.name, "offset": 0, "done": out.done, "state": out.state })))
}

async fn files_put(
    State(s): State<Arc<Shared>>,
    TrustedPeer(dev): TrustedPeer,
    UrlPath(id): UrlPath<String>,
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
    Ok(Json(json!({ "received": out.received, "done": out.done })))
}

async fn files_get(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    s.incoming_status(&dev.id, &id).map(Json).ok_or_else(ApiError::not_found)
}

async fn files_delete(State(s): State<Arc<Shared>>, TrustedPeer(dev): TrustedPeer, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    s.cancel_incoming(&dev.id, &id)?;
    Ok(Json(json!({ "cancelled": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_depends_on_every_input_and_on_roles() {
        let (fa, fb, fm) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        let (na, nb) = ([1u8; 16], [2u8; 16]);
        let base = pair_code(&fa, &fb, &na, &nb);
        assert_eq!(base.len(), 7, "six digits and a space: {base}");
        assert_eq!(base, pair_code(&fa, &fb, &na, &nb));
        assert_ne!(base, pair_code(&fb, &fa, &na, &nb), "roles matter");
        assert_ne!(base, pair_code(&fa, &fm, &na, &nb), "a man in the middle has another certificate");
        assert_ne!(base, pair_code(&fa, &fb, &[3u8; 16], &nb));
        assert_ne!(base, pair_code(&fa, &fb, &na, &[3u8; 16]));
    }
}
