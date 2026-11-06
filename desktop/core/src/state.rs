//! Shared state behind a `Bridge`: pairing, devices, note history, download tickets.
//!
//! All locks are plain `std::sync::Mutex` and are never held across an `.await`.

use std::path::PathBuf;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, watch};

use crate::error::Result;
use crate::identity::Identity;
use crate::model::*;
use crate::transfer::Incoming;
use crate::util::{now_ts, random_hex, write_private_atomic};

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/1/I
const CODE_LENGTH: usize = 10; // 32^10 = 50 bits
const MAX_DEVICES: usize = 20;
pub const MAX_TEXT_CHARS: usize = 100_000;
const HISTORY: usize = 500;
const TICKET_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    Invalid,
    Locked,
    TooManyDevices,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NoteError {
    Empty,
    TooLong,
}

#[derive(Default)]
struct PairingState {
    codes: HashMap<String, Instant>,
    fails: u32,
    locked_until: Option<Instant>,
}

#[derive(Default)]
struct TransferBook {
    map: HashMap<String, TransferInfo>,
    order: VecDeque<String>,
}

/// Messages deleted from the phone conversation, so a phone that polls can drop them too.
#[derive(Default)]
struct DeleteLog {
    seq: u64,
    /// (sequence number, phone, message id)
    items: VecDeque<(u64, String, String)>,
}

const DELETE_LOG: usize = 1000;

struct Ticket {
    device_id: String,
    file_id: String,
    expires: Instant,
}

pub(crate) struct Shared {
    pub cfg: Config,
    pub fingerprint: String,
    pub lan_host: String,
    pub lan_port: u16,
    pub peer_port: u16,
    pub identity: Arc<Identity>,
    pub peer: crate::peer::PeerState,
    /// The runtime of the bridge. Methods the window calls from its own thread (not a Tokio thread) start
    /// background work through this, because `tokio::spawn` panics outside a runtime.
    pub(crate) rt: tokio::runtime::Handle,
    /// Folder for received files; can be changed while running.
    inbox: Mutex<PathBuf>,
    pub(crate) persist: Mutex<Persist>,
    pairing: Mutex<PairingState>,
    last_seen: Mutex<HashMap<String, f64>>,
    pub incoming: Mutex<HashMap<String, Incoming>>,
    book: Mutex<TransferBook>,
    deleted: Mutex<DeleteLog>,
    tickets: Mutex<HashMap<String, Ticket>>,
    /// Counter bumped on every change a long-poll should wake for.
    wake: watch::Sender<u64>,
    events: broadcast::Sender<Event>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding a lock must not take the whole agent down with it.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A display name supplied by another device: no control characters, trimmed, at most 40 characters.
pub(crate) fn clean_name(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

/// Older versions kept one conversation for all phones. Give each old message to the phone it belonged to:
/// a message from a phone to the phone with that name, a message from this PC (it went to every phone) to
/// the phone that was paired first, which is nearly always the only one there was. Whatever cannot be
/// assigned is dropped, so no phone ever receives a message that was not meant for it.
fn migrate_phone_notes(p: &mut Persist) {
    let first = p.devices.iter().min_by(|a, b| a.1.created.total_cmp(&b.1.created)).map(|(id, _)| id.clone());
    for n in p.notes.iter_mut().filter(|n| n.dev.is_empty()) {
        let named: Vec<&String> = p.devices.iter().filter(|(_, d)| d.name == n.src).map(|(id, _)| id).collect();
        n.dev = match (n.src.as_str(), named.as_slice()) {
            ("pc", _) | (_, []) => first.clone().unwrap_or_default(),
            (_, [one]) => (*one).clone(),
            _ => first.clone().unwrap_or_default(),
        };
    }
    p.notes.retain(|n| !n.dev.is_empty() && p.devices.contains_key(&n.dev));
    let owners: std::collections::HashMap<String, String> =
        p.notes.iter().filter_map(|n| n.file.as_ref().map(|f| (f.id.clone(), n.dev.clone()))).collect();
    for (id, o) in p.offers.iter_mut() {
        if o.dev.is_empty() {
            o.dev = owners.get(id).cloned().unwrap_or_default();
        }
    }
    p.offers.retain(|_, o| !o.dev.is_empty());
}

/// The quote for a reply to `mid`, from our own copy of that message.
pub(crate) fn reply_for(notes: &[Note], mid: &str) -> Option<Reply> {
    let n = notes.iter().find(|n| n.mid == mid)?;
    let preview: String = match (&n.text, &n.file) {
        (Some(t), _) => t.chars().take(160).collect(),
        (None, Some(f)) => f.name.chars().take(160).collect(),
        _ => String::new(),
    };
    Some(Reply { mid: n.mid.clone(), src: n.src.clone(), preview })
}

/// Delete a received file from the inbox, if it is still where we saved it. Only ever called for
/// files this program put there.
pub(crate) fn delete_saved_file(inbox: &std::path::Path, f: &FileRef) {
    if let Some(path) = f.local_path.as_deref().map(std::path::Path::new) {
        if path.starts_with(inbox) {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

impl Shared {
    pub fn new(cfg: Config, identity: Arc<Identity>, lan_host: String, lan_port: u16, peer_port: u16) -> Result<Arc<Self>> {
        let fingerprint = identity.fingerprint.clone();
        std::fs::create_dir_all(&cfg.data_dir)?;
        let mut persist: Persist = std::fs::read(cfg.data_dir.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Notes saved by older versions have no message id yet.
        for n in persist.notes.iter_mut().chain(persist.peer_notes.values_mut().flatten()) {
            if n.mid.is_empty() {
                n.mid = n.file.as_ref().map_or_else(|| format!("old{}", n.id), |f| f.id.clone());
            }
        }
        migrate_phone_notes(&mut persist);
        let max_id = persist.notes.iter().map(|n| n.id).chain(persist.peer_notes.values().flatten().map(|n| n.id)).max().unwrap_or(0);
        persist.next_id = persist.next_id.max(max_id + 1);
        let (wake, _) = watch::channel(0);
        let (events, _) = broadcast::channel(256);
        let inbox = cfg.inbox_dir.clone();
        Ok(Arc::new(Self {
            cfg,
            fingerprint,
            lan_host,
            lan_port,
            peer_port,
            identity,
            peer: crate::peer::PeerState::default(),
            rt: tokio::runtime::Handle::current(),
            inbox: Mutex::new(inbox),
            persist: Mutex::new(persist),
            pairing: Mutex::new(PairingState::default()),
            last_seen: Mutex::new(HashMap::new()),
            incoming: Mutex::new(HashMap::new()),
            book: Mutex::new(TransferBook::default()),
            deleted: Mutex::new(DeleteLog::default()),
            tickets: Mutex::new(HashMap::new()),
            wake,
            events,
        }))
    }

    pub fn inbox(&self) -> PathBuf {
        lock(&self.inbox).clone()
    }

    pub fn set_inbox(&self, dir: PathBuf) {
        *lock(&self.inbox) = dir;
    }

    pub(crate) fn save(&self, p: &Persist) {
        let path = self.cfg.data_dir.join("state.json");
        match serde_json::to_vec(p) {
            Ok(bytes) => {
                if let Err(e) = write_private_atomic(&path, &bytes) {
                    tracing::error!("could not save state: {e}");
                }
            }
            Err(e) => tracing::error!("could not encode state: {e}"),
        }
    }

    pub fn emit(&self, ev: Event) {
        let _ = self.events.send(ev); // no subscribers is fine
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn wake_receiver(&self) -> watch::Receiver<u64> {
        self.wake.subscribe()
    }

    fn wake_pollers(&self) {
        self.wake.send_modify(|v| *v = v.wrapping_add(1));
    }

    // ------------------------------------------------------------ pairing

    pub fn new_pair_code(&self) -> String {
        let mut st = lock(&self.pairing);
        let now = Instant::now();
        st.codes.retain(|_, exp| *exp > now);
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let code: String =
            (0..CODE_LENGTH).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect();
        st.codes.insert(code.clone(), now + self.cfg.code_ttl);
        code
    }

    /// Exchange a one-time code for a device token. Returns (device id, token).
    pub fn pair(&self, code: &str, name: &str) -> std::result::Result<(String, String), PairError> {
        let now = Instant::now();
        {
            let mut st = lock(&self.pairing);
            if st.locked_until.is_some_and(|t| now < t) {
                return Err(PairError::Locked);
            }
            st.codes.retain(|_, exp| *exp > now);
            let supplied = normalize_code(code);
            let mut hit: Option<String> = None;
            for known in st.codes.keys() {
                // compare against every live code, no early exit
                if bool::from(known.as_bytes().ct_eq(supplied.as_bytes())) {
                    hit = Some(known.clone());
                }
            }
            let Some(hit) = hit else {
                st.fails += 1;
                if st.fails >= self.cfg.max_pair_fails {
                    st.codes.clear();
                    st.fails = 0;
                    st.locked_until = Some(now + self.cfg.lockout);
                }
                return Err(PairError::Invalid);
            };
            if lock(&self.persist).devices.len() >= MAX_DEVICES {
                return Err(PairError::TooManyDevices);
            }
            st.codes.remove(&hit); // single use
            st.fails = 0;
        }

        let name = clean_name(name);
        let name = if name.is_empty() { "Phone".to_string() } else { name };
        let token = random_hex(32);
        let id = random_hex(6);
        {
            let mut p = lock(&self.persist);
            p.devices.insert(id.clone(), DeviceRec { name, token_hash: hash_token(&token), created: now_ts() });
            self.save(&p);
        }
        self.emit(Event::DevicesChanged);
        Ok((id, token))
    }

    // ------------------------------------------------------------ devices

    pub fn auth(&self, token: &str) -> Option<Device> {
        if token.is_empty() || token.len() > 200 {
            return None;
        }
        let want = hash_token(token);
        let p = lock(&self.persist);
        let mut found: Option<&String> = None;
        for (id, rec) in &p.devices {
            if bool::from(rec.token_hash.as_bytes().ct_eq(want.as_bytes())) {
                found = Some(id);
            }
        }
        let id = found?.clone();
        let name = p.devices[&id].name.clone();
        let created = p.devices[&id].created;
        drop(p);
        let now = now_ts();
        lock(&self.last_seen).insert(id.clone(), now);
        Some(Device { id, name, created, last_seen: Some(now) })
    }

    pub fn device_name(&self, id: &str) -> Option<String> {
        lock(&self.persist).devices.get(id).map(|d| d.name.clone())
    }

    pub fn device_exists(&self, id: &str) -> bool {
        lock(&self.persist).devices.contains_key(id)
    }

    pub fn rename_device(&self, id: &str, name: &str) -> bool {
        let name = clean_name(name);
        if name.is_empty() {
            return false;
        }
        {
            let mut p = lock(&self.persist);
            let Some(rec) = p.devices.get_mut(id) else { return false };
            rec.name = name;
            self.save(&p);
        }
        self.emit(Event::DevicesChanged);
        true
    }

    pub fn devices(&self) -> Vec<Device> {
        let p = lock(&self.persist);
        let seen = lock(&self.last_seen);
        p.devices
            .iter()
            .map(|(id, d)| Device { id: id.clone(), name: d.name.clone(), created: d.created, last_seen: seen.get(id).copied() })
            .collect()
    }

    pub fn revoke(&self, id: &str) -> bool {
        let removed = {
            let mut p = lock(&self.persist);
            let existed = p.devices.remove(id).is_some();
            if existed {
                // the conversation with an unpaired phone goes with it (files already received stay in the inbox)
                p.notes.retain(|n| n.dev != id);
                p.offers.retain(|_, o| o.dev != id);
                self.save(&p);
            }
            existed
        };
        if !removed {
            return false;
        }
        lock(&self.last_seen).remove(id);
        lock(&self.tickets).retain(|_, t| t.device_id != id);
        // Half-received files from this phone are useless once it is unpaired.
        let gone: Vec<_> = {
            let mut inc = lock(&self.incoming);
            let ids: Vec<String> = inc.iter().filter(|(_, i)| i.device_id == id).map(|(k, _)| k.clone()).collect();
            ids.iter().filter_map(|k| inc.remove(k)).collect()
        };
        for i in gone {
            crate::transfer::remove_incoming_files(&self.cfg, &i.id);
        }
        self.emit(Event::DevicesChanged);
        self.wake_pollers(); // long-polls re-check auth and answer 401
        true
    }

    // ------------------------------------------------------------ notes

    /// `dev`: the phone whose conversation this belongs to; `src`: "pc" or that phone's name.
    /// `reply_to`: the message id this note answers (looked up in the same conversation).
    pub fn add_text_note(&self, dev: &str, src: &str, text: &str, reply_to: Option<&str>) -> std::result::Result<Note, NoteError> {
        if text.trim().is_empty() {
            return Err(NoteError::Empty);
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(NoteError::TooLong);
        }
        Ok(self.push_note(dev, src, NoteKind::Text, random_hex(8), Some(text.to_string()), None, reply_to))
    }

    pub fn add_file_note(&self, dev: &str, src: &str, file: FileRef) -> Note {
        let mid = file.id.clone();
        self.push_note(dev, src, NoteKind::File, mid, None, Some(file), None)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_note(&self, dev: &str, src: &str, kind: NoteKind, mid: String, text: Option<String>, file: Option<FileRef>, reply_to: Option<&str>) -> Note {
        let note = {
            let mut p = lock(&self.persist);
            let mine: Vec<Note> = p.notes.iter().filter(|n| n.dev == dev).cloned().collect();
            let reply = reply_to.and_then(|m| reply_for(&mine, m));
            let note = Note { id: p.next_id, mid, ts: now_ts(), src: src.to_string(), dev: dev.to_string(), kind, text, file, reply, chat_hidden: false, list_hidden: false, pending: false };
            p.next_id += 1;
            p.notes.push(note.clone());
            // the history limit counts per phone, so a chatty phone cannot push out another one's messages
            let count = p.notes.iter().filter(|n| n.dev == dev).count();
            if count > HISTORY {
                let mut drop_n = count - HISTORY;
                p.notes.retain(|n| {
                    if n.dev == dev && drop_n > 0 {
                        drop_n -= 1;
                        false
                    } else {
                        true
                    }
                });
                // forget offers that no remaining note refers to
                let keep: HashSet<String> =
                    p.notes.iter().filter_map(|n| n.file.as_ref().map(|f| f.id.clone())).collect();
                p.offers.retain(|id, _| keep.contains(id));
            }
            self.save(&p);
            note
        };
        self.emit(Event::NoteAdded(note.clone()));
        self.wake_pollers();
        note
    }

    /// The conversation with phone `dev`. With `since = None`, the most recent `limit` notes; otherwise
    /// notes newer than `since`. Notes of other phones are never included.
    pub fn notes_since(&self, dev: &str, since: Option<u64>, limit: usize) -> Vec<Note> {
        let p = lock(&self.persist);
        match since {
            None => {
                let mut v: Vec<Note> = p.notes.iter().rev().filter(|n| n.dev == dev).take(limit).cloned().collect();
                v.reverse();
                v
            }
            Some(s) => p.notes.iter().filter(|n| n.dev == dev && n.id > s).take(limit).cloned().collect(),
        }
    }

    /// Clear the chat, the list "Files" or both for the conversation with one phone (the other phones keep
    /// theirs). The two views are independent. This only hides messages on this PC: the phone keeps its
    /// history, and a file offered to it stays available. Messages are removed by deleting them one by one
    /// (for both) or by unpairing the phone.
    pub fn clear_notes(&self, dev: &str, what: Clear, remove: bool) {
        let mut p = lock(&self.persist);
        // Without `remove` the notes stay (only hidden on this PC): the phone reads its history from here.
        let gone = what.apply(&mut p.notes, |n| n.dev == dev, remove);
        for f in gone.iter().filter_map(|n| n.file.as_ref()) {
            p.offers.remove(&f.id);
        }
        self.save(&p);
    }

    /// Delete one message from the conversation with phone `dev`. A file received from a phone is deleted
    /// from the inbox too; a file offered to the phone stops being available (the original is never touched).
    /// A message of another phone is not found.
    pub fn delete_note(&self, dev: &str, mid: &str) -> bool {
        let removed = {
            let mut p = lock(&self.persist);
            let Some(pos) = p.notes.iter().position(|n| n.mid == mid && n.dev == dev) else { return false };
            let note = p.notes.remove(pos);
            if let Some(f) = &note.file {
                p.offers.remove(&f.id);
                if f.dir == Direction::ToPc {
                    delete_saved_file(&self.inbox(), f);
                }
            }
            self.save(&p);
            note
        };
        self.log_deleted(dev, vec![removed.mid.clone()]);
        self.emit(Event::NoteRemoved { peer_id: None, mid: removed.mid });
        self.wake_pollers();
        true
    }

    fn log_deleted(&self, dev: &str, mids: Vec<String>) {
        let mut d = lock(&self.deleted);
        for m in mids {
            d.seq += 1;
            let seq = d.seq;
            d.items.push_back((seq, dev.to_string(), m));
        }
        while d.items.len() > DELETE_LOG {
            d.items.pop_front();
        }
    }

    /// Message ids deleted from the conversation with phone `dev` after `seq`, and the current sequence number.
    pub fn deleted_since(&self, dev: &str, seq: u64) -> (Vec<String>, u64) {
        let d = lock(&self.deleted);
        (d.items.iter().filter(|(s, w, _)| *s > seq && w == dev).map(|(_, _, m)| m.clone()).collect(), d.seq)
    }

    pub fn auto_accept_files(&self) -> bool {
        lock(&self.persist).auto_accept_files
    }

    pub fn set_auto_accept_files(&self, on: bool) {
        let mut p = lock(&self.persist);
        if p.auto_accept_files != on {
            p.auto_accept_files = on;
            self.save(&p);
        }
    }

    /// Seconds after which an unaccepted file offer to another PC is withdrawn; 0 means never.
    pub fn offer_timeout_secs(&self) -> u32 {
        lock(&self.persist).offer_timeout_secs
    }

    pub fn set_offer_timeout_secs(&self, secs: u32) {
        let mut p = lock(&self.persist);
        if p.offer_timeout_secs != secs {
            p.offer_timeout_secs = secs;
            self.save(&p);
        }
    }

    // ------------------------------------------------------------ offers (PC -> phone files)

    pub fn put_offer(&self, rec: OfferRec) {
        let mut p = lock(&self.persist);
        p.offers.insert(rec.id.clone(), rec);
        self.save(&p);
    }

    /// The file `id` as far as phone `dev` may see it: only the phone it was offered to.
    pub fn offer_for(&self, dev: &str, id: &str) -> Option<OfferRec> {
        lock(&self.persist).offers.get(id).filter(|o| o.dev == dev).cloned()
    }

    // ------------------------------------------------------------ download tickets

    /// A short-lived link token so a browser can download without an Authorization header.
    pub fn new_ticket(&self, device_id: &str, file_id: &str) -> String {
        let mut t = lock(&self.tickets);
        let now = Instant::now();
        t.retain(|_, v| v.expires > now);
        let ticket = random_hex(16);
        t.insert(
            ticket.clone(),
            Ticket { device_id: device_id.to_string(), file_id: file_id.to_string(), expires: now + TICKET_TTL },
        );
        ticket
    }

    /// Device id behind a valid, unexpired ticket for this file.
    pub fn check_ticket(&self, ticket: &str, file_id: &str) -> Option<Device> {
        let device_id = {
            let t = lock(&self.tickets);
            let tk = t.get(ticket)?;
            if tk.expires <= Instant::now() || tk.file_id != file_id {
                return None;
            }
            tk.device_id.clone()
        };
        let p = lock(&self.persist);
        let rec = p.devices.get(&device_id)?;
        Some(Device { id: device_id, name: rec.name.clone(), created: rec.created, last_seen: None })
    }

    // ------------------------------------------------------------ transfer registry

    pub fn set_transfer(&self, info: TransferInfo) {
        {
            let mut b = lock(&self.book);
            if !b.map.contains_key(&info.id) {
                b.order.push_back(info.id.clone());
            }
            b.map.insert(info.id.clone(), info.clone());
            while b.order.len() > 60 {
                let pos = b.order.iter().position(|id| {
                    b.map.get(id).is_none_or(|t| !matches!(t.state, TransferState::Active | TransferState::Hashing | TransferState::Offered))
                });
                match pos {
                    Some(i) => {
                        if let Some(id) = b.order.remove(i) {
                            b.map.remove(&id);
                        }
                    }
                    None => break,
                }
            }
        }
        self.emit(Event::Transfer(info));
    }

    pub fn transfers(&self) -> Vec<TransferInfo> {
        let b = lock(&self.book);
        b.order.iter().filter_map(|id| b.map.get(id).cloned()).collect()
    }

    pub fn forget_transfer(&self, id: &str) {
        let mut b = lock(&self.book);
        b.map.remove(id);
        b.order.retain(|x| x != id);
    }
}

pub(crate) fn lock_mutex<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    lock(m)
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use crate::model::{DeviceRec, FileRef, OfferRec};

    fn note(id: u64, src: &str, text: &str) -> Note {
        Note { id, mid: format!("m{id}"), ts: 0.0, src: src.into(), dev: String::new(), kind: NoteKind::Text, text: Some(text.into()), file: None, reply: None, chat_hidden: false, list_hidden: false, pending: false }
    }

    fn device(name: &str, created: f64) -> DeviceRec {
        DeviceRec { name: name.into(), token_hash: String::new(), created }
    }

    #[test]
    fn old_shared_notes_are_split_between_the_phones_that_had_them() {
        let mut p = Persist::default();
        p.devices.insert("first".into(), device("Pixel", 1.0));
        p.devices.insert("second".into(), device("Galaxy", 2.0));
        p.notes = vec![note(1, "Pixel", "p1"), note(2, "pc", "to everybody"), note(3, "Galaxy", "g1"), note(4, "Gone phone", "orphan")];
        let file = FileRef { id: "f1".into(), name: "a.txt".into(), size: 1, sha256: String::new(), dir: crate::model::Direction::ToPhone, local_path: None };
        let mut n5 = note(5, "pc", "");
        n5.kind = NoteKind::File;
        n5.file = Some(file);
        p.notes.push(n5);
        p.offers.insert("f1".into(), OfferRec { id: "f1".into(), dev: String::new(), name: "a.txt".into(), size: 1, sha256: String::new(), path: "/x".into(), mtime_ns: 0 });
        p.offers.insert("lost".into(), OfferRec { id: "lost".into(), dev: String::new(), name: "b".into(), size: 1, sha256: String::new(), path: "/y".into(), mtime_ns: 0 });

        migrate_phone_notes(&mut p);

        let of = |dev: &str| -> Vec<&str> { p.notes.iter().filter(|n| n.dev == dev).map(|n| n.mid.as_str()).collect() };
        // a message of a phone goes to that phone; a message of the PC (and one from an unknown phone) to the first
        assert_eq!(of("second"), ["m3"]);
        assert_eq!(of("first"), ["m1", "m2", "m4", "m5"]);
        assert!(p.notes.iter().all(|n| !n.dev.is_empty()));
        assert_eq!(p.offers["f1"].dev, "first");
        assert!(!p.offers.contains_key("lost"), "an offer nobody owns is dropped");
    }

    #[test]
    fn without_any_phone_old_notes_are_dropped() {
        let mut p = Persist { notes: vec![note(1, "pc", "x")], ..Persist::default() };
        migrate_phone_notes(&mut p);
        assert!(p.notes.is_empty());
    }
}
