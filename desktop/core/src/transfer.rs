//! File transfer: resumable uploads from the phone, ranged downloads to the phone.
//!
//! Upload  (phone -> PC): create (`POST /api/files`), then one or more `PUT ...?offset=N`.
//!         Bytes stream straight to `<data>/incoming/<id>.part`. The phone can always ask how
//!         much arrived and continue from there, also after the PC restarted.
//! Download (PC -> phone): the PC "offers" a file (checksum computed first); the phone fetches it
//!         with ordinary HTTP `Range` requests, so a broken download resumes where it stopped.

use std::io::{Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::StatusCode;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

use crate::error::{ApiError, Error};
use crate::model::*;
use crate::state::{lock_mutex as lock, Shared};
use crate::util::{now_ts, random_hex, reserve_unique, sanitize_name};

const FLUSH_BYTES: usize = 1024 * 1024;
const READ_CHUNK: usize = 256 * 1024;
const MAX_OPEN_UPLOADS_PER_DEVICE: usize = 16;
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
const PART_MAX_AGE_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// An offer nobody answered is dropped after this long.
const PENDING_TTL_SECS: f64 = 15.0 * 60.0;
/// A declined or cancelled transfer is remembered this long so the sender can still ask about it.
const GONE_TTL_SECS: f64 = 30.0 * 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InState {
    /// Announced by the sender, waiting for the user. Nothing is on disk yet.
    Pending,
    Receiving,
    Finalizing,
    Done,
    Declined,
    Cancelled,
}

pub(crate) struct Incoming {
    pub id: String,
    /// True when the sender is another PC rather than a phone.
    pub peer: bool,
    pub device_id: String,
    pub device_name: String,
    pub name: String,
    pub size: u64,
    pub claimed_sha: Option<String>,
    pub received: u64,
    pub busy: bool,
    pub state: InState,
    pub created: f64,
}

#[derive(Serialize, Deserialize)]
struct Sidecar {
    id: String,
    #[serde(default)]
    peer: bool,
    device_id: String,
    device_name: String,
    name: String,
    size: u64,
    sha256: Option<String>,
    created: f64,
}

pub struct CreateOut {
    pub id: String,
    pub name: String,
    /// "pending" (waiting for the user), "receiving" or "done".
    pub state: &'static str,
    pub done: bool,
    pub file: Option<FileRef>,
}

pub struct PutOut {
    pub received: u64,
    pub done: bool,
    pub file: Option<FileRef>,
}

fn incoming_dir(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("incoming")
}
fn part_path(cfg: &Config, id: &str) -> PathBuf {
    incoming_dir(cfg).join(format!("{id}.part"))
}
fn meta_path(cfg: &Config, id: &str) -> PathBuf {
    incoming_dir(cfg).join(format!("{id}.json"))
}

pub(crate) fn remove_incoming_files(cfg: &Config, id: &str) {
    let _ = std::fs::remove_file(part_path(cfg, id));
    let _ = std::fs::remove_file(meta_path(cfg, id));
}

/// SHA-256 of a file, streamed in 1 MiB blocks.
pub(crate) fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn mtime_ns(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Rename, falling back to copy+delete when the inbox is on another drive.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

struct BusyGuard {
    shared: Arc<Shared>,
    id: String,
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Some(i) = lock(&self.shared.incoming).get_mut(&self.id) {
            i.busy = false;
        }
    }
}

impl Shared {
    fn upload_info(&self, i: &Incoming, state: TransferState, done: u64, error: Option<&str>) -> TransferInfo {
        TransferInfo {
            id: i.id.clone(),
            name: i.name.clone(),
            size: i.size,
            done,
            dir: if i.peer { Direction::FromPeer } else { Direction::ToPc },
            state,
            device: i.device_name.clone(),
            device_id: i.device_id.clone(),
            error: error.map(str::to_string),
        }
    }

    // ------------------------------------------------------------ upload: create

    pub(crate) async fn create_incoming(
        self: &Arc<Self>,
        dev: &Device,
        name: &str,
        size: u64,
        sha: Option<String>,
    ) -> Result<CreateOut, ApiError> {
        self.create_incoming_for(dev, false, name, size, sha, None).await
    }

    /// `peer`: the sender is a paired PC, so the finished file is announced in that PC's conversation.
    /// `want_id`: the id the sender picked (16 hex characters), so both sides call the transfer the same.
    ///
    /// Unless the user turned on "receive without asking", nothing is stored yet: the transfer waits as
    /// `Offered` until `accept_incoming`.
    pub(crate) async fn create_incoming_for(
        self: &Arc<Self>,
        dev: &Device,
        peer: bool,
        name: &str,
        size: u64,
        sha: Option<String>,
        want_id: Option<String>,
    ) -> Result<CreateOut, ApiError> {
        if size > self.cfg.max_file_size {
            return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "file_too_large"));
        }
        let sha = match sha {
            None => None,
            Some(s) if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) => Some(s.to_ascii_lowercase()),
            Some(_) => return Err(ApiError::bad_request("bad_sha256")),
        };
        let id = match want_id {
            None => random_hex(8),
            Some(w) if w.len() == 16 && w.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => w,
            Some(_) => return Err(ApiError::bad_request("bad_id")),
        };
        self.forget_old_incoming();
        {
            let inc = lock(&self.incoming);
            if inc.contains_key(&id) {
                return Err(ApiError::conflict("id_in_use"));
            }
            let open = inc.values().filter(|i| i.device_id == dev.id && matches!(i.state, InState::Receiving | InState::Pending)).count();
            if open >= MAX_OPEN_UPLOADS_PER_DEVICE {
                return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_transfers"));
            }
        }
        let item = Incoming {
            id: id.clone(),
            peer,
            device_id: dev.id.clone(),
            device_name: dev.name.clone(),
            name: sanitize_name(name),
            size,
            claimed_sha: sha,
            received: 0,
            busy: false,
            state: InState::Pending,
            created: now_ts(),
        };
        let info = self.upload_info(&item, TransferState::Offered, 0, None);
        lock(&self.incoming).insert(id.clone(), item);
        self.set_transfer(info);

        if self.auto_accept_files() {
            return self.accept_incoming(&id).await.map(|file| CreateOut {
                id: id.clone(),
                name: file.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
                state: if file.is_some() { "done" } else { "receiving" },
                done: file.is_some(),
                file,
            });
        }
        let name_out = lock(&self.incoming).get(&id).map(|i| i.name.clone()).unwrap_or_default();
        Ok(CreateOut { id, name: name_out, state: "pending", done: false, file: None })
    }

    /// The user said yes (or "receive without asking" is on): create the `.part` file and start receiving.
    /// Returns the finished file for a zero-byte transfer, which needs no upload.
    pub(crate) async fn accept_incoming(self: &Arc<Self>, id: &str) -> Result<Option<FileRef>, ApiError> {
        let side = {
            let inc = lock(&self.incoming);
            let it = inc.get(id).ok_or_else(ApiError::not_found)?;
            if it.state != InState::Pending {
                return Err(ApiError::conflict("not_pending"));
            }
            Sidecar {
                id: it.id.clone(),
                peer: it.peer,
                device_id: it.device_id.clone(),
                device_name: it.device_name.clone(),
                name: it.name.clone(),
                size: it.size,
                sha256: it.claimed_sha.clone(),
                created: it.created,
            }
        };
        let dir = incoming_dir(&self.cfg);
        std::fs::create_dir_all(&dir).map_err(|e| ApiError::from_io(&e))?;
        crate::util::open_private(&part_path(&self.cfg, id)).map_err(|e| ApiError::from_io(&e))?;
        let bytes = serde_json::to_vec(&side).map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal"))?;
        crate::util::write_private_atomic(&meta_path(&self.cfg, id), &bytes).map_err(|e| ApiError::from_io(&e))?;
        let info = {
            let mut inc = lock(&self.incoming);
            let it = match inc.get_mut(id) {
                Some(it) if it.state == InState::Pending => it,
                _ => {
                    drop(inc);
                    remove_incoming_files(&self.cfg, id); // cancelled while we were creating the files
                    return Err(ApiError::conflict("not_pending"));
                }
            };
            it.state = InState::Receiving;
            self.upload_info(it, TransferState::Active, 0, None)
        };
        let size = info.size;
        self.set_transfer(info);
        if size == 0 {
            return Ok(Some(self.finalize(id).await?));
        }
        Ok(None)
    }

    /// The user said no. The sender finds out when it asks.
    pub(crate) fn decline_incoming(&self, id: &str) -> bool {
        self.stop_incoming(id, InState::Declined, None, &[InState::Pending])
    }

    /// The user stops a transfer that is running (or still waiting).
    pub(crate) fn cancel_receiving(&self, id: &str) -> bool {
        self.stop_incoming(id, InState::Cancelled, None, &[InState::Pending, InState::Receiving])
    }

    /// Move an incoming transfer to `to` if it is in one of the `from` states, and tell the UI.
    /// Declined offers disappear from this PC's list at once; cancelled ones stay until removed.
    fn stop_incoming(&self, id: &str, to: InState, error: Option<String>, from: &[InState]) -> bool {
        let (info, busy) = {
            let mut inc = lock(&self.incoming);
            let Some(it) = inc.get_mut(id) else { return false };
            if !from.contains(&it.state) {
                return false;
            }
            it.state = to;
            it.created = now_ts(); // forgotten GONE_TTL after this
            let state = if to == InState::Declined { TransferState::Declined } else { TransferState::Cancelled };
            (self.upload_info(it, state, it.received, error.as_deref()), it.busy)
        };
        // A running upload closes its file first and deletes the files itself.
        if !busy {
            remove_incoming_files(&self.cfg, id);
        }
        if to == InState::Declined {
            self.forget_transfer(id);
        } else {
            self.set_transfer(info);
        }
        true
    }

    /// Remove declined and cancelled transfers that nobody asked about for a while.
    fn forget_old_incoming(&self) {
        let now = now_ts();
        let mut expired: Vec<String> = Vec::new();
        {
            let mut inc = lock(&self.incoming);
            inc.retain(|id, i| {
                let gone = matches!(i.state, InState::Declined | InState::Cancelled) && now - i.created > GONE_TTL_SECS;
                if gone {
                    expired.push(id.clone());
                }
                !gone
            });
            let stale: Vec<String> = inc.values().filter(|i| i.state == InState::Pending && now - i.created > PENDING_TTL_SECS).map(|i| i.id.clone()).collect();
            drop(inc);
            for id in stale {
                self.stop_incoming(&id, InState::Cancelled, Some("Nobody answered in time".into()), &[InState::Pending]);
            }
        }
        for id in expired {
            self.forget_transfer(&id);
        }
    }

    /// Take a finished, cancelled or declined transfer off the list. For another PC's transfer `peer`
    /// is its fingerprint; a mismatch is refused. Returns whether something was removed.
    pub(crate) fn dismiss_transfer(&self, id: &str, peer: Option<&str>) -> bool {
        let known = self.transfers().into_iter().find(|t| t.id == id);
        let state_ok = |s: TransferState| matches!(s, TransferState::Done | TransferState::Declined | TransferState::Cancelled | TransferState::Failed);
        let mut removed = false;
        {
            let mut inc = lock(&self.incoming);
            if let Some(i) = inc.get(id) {
                let mine = peer.is_none_or(|p| i.device_id == p);
                if mine && matches!(i.state, InState::Declined | InState::Cancelled | InState::Done) {
                    inc.remove(id);
                    removed = true;
                }
            }
        }
        if let Some(t) = known {
            let mine = peer.is_none_or(|p| t.device_id == p);
            if mine && state_ok(t.state) {
                removed = true;
            }
        }
        if removed {
            self.forget_transfer(id);
        }
        removed
    }

    // ------------------------------------------------------------ upload: put bytes

    pub(crate) async fn put_incoming(
        self: &Arc<Self>,
        dev: &Device,
        id: &str,
        offset: u64,
        content_length: Option<u64>,
        body: Body,
    ) -> Result<PutOut, ApiError> {
        let (size, mut info) = {
            let mut inc = lock(&self.incoming);
            let it = inc.get_mut(id).filter(|i| i.device_id == dev.id).ok_or_else(ApiError::not_found)?;
            match it.state {
                InState::Receiving => {}
                InState::Pending => return Err(ApiError::conflict("not_accepted")),
                InState::Declined => return Err(ApiError::conflict("declined")),
                InState::Cancelled => return Err(ApiError::conflict("cancelled")),
                _ => return Err(ApiError::conflict("not_receiving")),
            }
            if it.busy {
                return Err(ApiError::conflict("busy"));
            }
            if it.received != offset {
                return Err(ApiError::conflict("offset_mismatch").with("received", it.received));
            }
            it.busy = true;
            (it.size, self.upload_info(it, TransferState::Active, it.received, None))
        };
        let _guard = BusyGuard { shared: self.clone(), id: id.to_string() };

        if content_length.is_some_and(|cl| offset.saturating_add(cl) > size) {
            return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_much_data"));
        }

        let part = part_path(&self.cfg, id);
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&part)
            .await
            .map_err(|e| ApiError::from_io(&e))?;
        file.set_len(offset).await.map_err(|e| ApiError::from_io(&e))?; // drop anything past the agreed offset
        file.seek(SeekFrom::Start(offset)).await.map_err(|e| ApiError::from_io(&e))?;

        let mut stream = body.into_data_stream();
        let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_BYTES + 64 * 1024);
        let mut received = offset;
        let mut last_emit = Instant::now();
        let mut failure: Option<ApiError> = None;

        loop {
            match tokio::time::timeout(self.cfg.idle_timeout, stream.next()).await {
                Err(_) => {
                    failure = Some(ApiError::new(StatusCode::REQUEST_TIMEOUT, "upload_stalled"));
                    break;
                }
                Ok(None) => break,
                Ok(Some(Err(_))) => {
                    failure = Some(ApiError::bad_request("body_error"));
                    break;
                }
                Ok(Some(Ok(chunk))) => {
                    if !self.is_receiving(id) {
                        failure = Some(ApiError::conflict("cancelled"));
                        break;
                    }
                    if received + buf.len() as u64 + chunk.len() as u64 > size {
                        failure = Some(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_much_data"));
                        break;
                    }
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= FLUSH_BYTES {
                        match flush(&mut file, &mut buf).await {
                            Ok(n) => received += n,
                            Err(e) => {
                                failure = Some(ApiError::from_io(&e));
                                break;
                            }
                        }
                        self.note_received(id, received);
                        if last_emit.elapsed() >= PROGRESS_EVERY {
                            info.done = received;
                            self.set_transfer(info.clone());
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }
        // Whatever arrived in order is valid; keep it so the phone can resume from there.
        match flush(&mut file, &mut buf).await {
            Ok(n) => received += n,
            Err(e) => failure = failure.or(Some(ApiError::from_io(&e))),
        }
        self.note_received(id, received);

        if let Some(err) = failure {
            if err.code == "cancelled" {
                // stopped by the user (either side): the canceller already updated the list
                drop(file);
                remove_incoming_files(&self.cfg, id);
                return Err(err);
            }
            info.done = received;
            info.state = TransferState::Interrupted;
            self.set_transfer(info);
            return Err(err.with("received", received));
        }

        if received < size {
            info.done = received;
            self.set_transfer(info);
            return Ok(PutOut { received, done: false, file: None });
        }

        file.sync_all().await.map_err(|e| ApiError::from_io(&e))?;
        drop(file);
        let fref = self.finalize(id).await?;
        Ok(PutOut { received, done: true, file: Some(fref) })
    }

    fn is_receiving(&self, id: &str) -> bool {
        lock(&self.incoming).get(id).is_some_and(|i| i.state == InState::Receiving)
    }

    fn note_received(&self, id: &str, received: u64) {
        if let Some(i) = lock(&self.incoming).get_mut(id) {
            i.received = received;
        }
    }

    fn fail_incoming(&self, id: &str, reason: &str) {
        let info = {
            let mut inc = lock(&self.incoming);
            inc.remove(id).map(|i| self.upload_info(&i, TransferState::Failed, i.received, Some(reason)))
        };
        remove_incoming_files(&self.cfg, id);
        if let Some(info) = info {
            self.set_transfer(info);
        }
    }

    /// All bytes are on disk: verify, move into the inbox, announce.
    async fn finalize(self: &Arc<Self>, id: &str) -> Result<FileRef, ApiError> {
        let (dev_name, name, size, claimed, peer, dev_id) = {
            let mut inc = lock(&self.incoming);
            let it = inc.get_mut(id).ok_or_else(ApiError::not_found)?;
            it.state = InState::Finalizing;
            (it.device_name.clone(), it.name.clone(), it.size, it.claimed_sha.clone(), it.peer, it.device_id.clone())
        };
        let part = part_path(&self.cfg, id);

        let p = part.clone();
        let hash = match tokio::task::spawn_blocking(move || hash_file(&p)).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                self.fail_incoming(id, "read_failed");
                return Err(ApiError::from_io(&e));
            }
            Err(_) => {
                self.fail_incoming(id, "internal");
                return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal"));
            }
        };
        if claimed.as_deref().is_some_and(|c| c != hash) {
            self.fail_incoming(id, "hash_mismatch");
            return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "hash_mismatch"));
        }

        let inbox = self.inbox();
        let p = part.clone();
        let moved = tokio::task::spawn_blocking(move || -> std::io::Result<PathBuf> {
            std::fs::create_dir_all(&inbox)?;
            let dest = reserve_unique(&inbox, &name)?;
            if let Err(e) = move_file(&p, &dest) {
                let _ = std::fs::remove_file(&dest);
                return Err(e);
            }
            Ok(dest)
        })
        .await;
        let dest = match moved {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => {
                self.fail_incoming(id, "save_failed");
                return Err(ApiError::from_io(&e));
            }
            Err(_) => {
                self.fail_incoming(id, "internal");
                return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal"));
            }
        };
        let final_name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = std::fs::remove_file(meta_path(&self.cfg, id));

        let info = {
            let mut inc = lock(&self.incoming);
            let it = inc.get_mut(id).ok_or_else(ApiError::not_found)?;
            it.state = InState::Done;
            it.name = final_name.clone();
            self.upload_info(it, TransferState::Done, size, None)
        };
        self.set_transfer(info);
        let local_path = Some(dest.to_string_lossy().into_owned());
        if peer {
            let fref = FileRef { id: id.to_string(), name: final_name, size, sha256: hash, dir: Direction::FromPeer, local_path };
            self.add_peer_note(&dev_id, &dev_name, NoteKind::File, id.to_string(), None, Some(fref.clone()), None);
            return Ok(fref);
        }
        let fref = FileRef { id: id.to_string(), name: final_name, size, sha256: hash, dir: Direction::ToPc, local_path };
        self.add_file_note(&dev_id, &dev_name, fref.clone());
        Ok(fref)
    }

    // ------------------------------------------------------------ upload: status / cancel

    pub(crate) fn incoming_status(&self, dev_id: &str, id: &str) -> Option<serde_json::Value> {
        self.forget_old_incoming(); // the sender asks often, which also expires offers nobody answered
        let inc = lock(&self.incoming);
        let i = inc.get(id).filter(|i| i.device_id == dev_id)?;
        let state = match i.state {
            InState::Pending => "pending",
            InState::Receiving => "receiving",
            InState::Finalizing => "finalizing",
            InState::Done => "done",
            InState::Declined => "declined",
            InState::Cancelled => "cancelled",
        };
        Some(serde_json::json!({
            "id": i.id, "name": i.name, "size": i.size, "received": i.received,
            "state": state, "dir": "to_pc",
        }))
    }

    /// The sender (phone or PC) stops its transfer. Safe to repeat.
    pub(crate) fn cancel_incoming(&self, dev_id: &str, id: &str) -> Result<(), ApiError> {
        let (state, name) = {
            let inc = lock(&self.incoming);
            let it = inc.get(id).filter(|i| i.device_id == dev_id).ok_or_else(ApiError::not_found)?;
            (it.state, it.device_name.clone())
        };
        match state {
            InState::Finalizing => Err(ApiError::conflict("busy")),
            InState::Done => Err(ApiError::conflict("already_done")),
            InState::Declined | InState::Cancelled => Ok(()),
            InState::Pending | InState::Receiving => {
                self.stop_incoming(id, InState::Cancelled, Some(format!("Cancelled by {name}")), &[InState::Pending, InState::Receiving]);
                Ok(())
            }
        }
    }

    /// Pick up half-received files after a restart so the phone can resume them.
    pub(crate) fn load_incoming(&self) {
        let dir = incoming_dir(&self.cfg);
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let now = now_ts();
        let mut known: Vec<String> = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(side) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Sidecar>(&b).ok()) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let part = part_path(&self.cfg, &side.id);
            let len = std::fs::metadata(&part).map(|m| m.len()).ok();
            let stale = now - side.created > PART_MAX_AGE_SECS;
            let valid_id = side.id.len() == 16 && side.id.bytes().all(|b| b.is_ascii_hexdigit());
            let known_sender = if side.peer { self.peer_exists(&side.device_id) } else { self.device_exists(&side.device_id) };
            if !valid_id || stale || len.is_none() || !known_sender {
                if valid_id {
                    remove_incoming_files(&self.cfg, &side.id);
                }
                continue;
            }
            let received = len.unwrap_or(0).min(side.size);
            let item = Incoming {
                id: side.id.clone(),
                peer: side.peer,
                device_id: side.device_id,
                device_name: side.device_name,
                name: side.name,
                size: side.size,
                claimed_sha: side.sha256,
                received,
                busy: false,
                state: InState::Receiving,
                created: side.created,
            };
            let info = self.upload_info(&item, TransferState::Interrupted, received, None);
            known.push(item.id.clone());
            lock(&self.incoming).insert(item.id.clone(), item);
            self.set_transfer(info);
        }
        // part files without a sidecar can never be resumed
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("part") {
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    if !known.iter().any(|k| k == stem) {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ offers (PC -> phone)

    /// Make a local file available to paired phones. Computes the checksum first, so a big
    /// file takes a moment; call it from a background task in a UI.
    pub async fn make_offer(self: &Arc<Self>, dev_id: &str, path: &Path) -> Result<FileRef, Error> {
        let dev_name = self.device_name(dev_id).ok_or_else(|| Error::Invalid("that phone is not paired".into()))?;
        let path = tokio::fs::canonicalize(path).await?;
        let meta = tokio::fs::metadata(&path).await?;
        if !meta.is_file() {
            return Err(Error::Invalid("not a regular file".into()));
        }
        let name = sanitize_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let id = random_hex(8);
        let mut info = TransferInfo {
            id: id.clone(),
            name: name.clone(),
            size: meta.len(),
            done: 0,
            dir: Direction::ToPhone,
            state: TransferState::Hashing,
            device: dev_name.clone(),
            device_id: dev_id.to_string(),
            error: None,
        };
        self.set_transfer(info.clone());

        let p = path.clone();
        let hashed = tokio::task::spawn_blocking(move || -> std::io::Result<(String, std::fs::Metadata)> {
            let h = hash_file(&p)?;
            Ok((h, std::fs::metadata(&p)?))
        })
        .await
        .map_err(|_| Error::Invalid("hashing task failed".into()))?;
        let (sha256, after) = match hashed {
            Ok(v) => v,
            Err(e) => {
                info.state = TransferState::Failed;
                info.error = Some(e.to_string());
                self.set_transfer(info);
                return Err(e.into());
            }
        };
        if after.len() != meta.len() || mtime_ns(&after) != mtime_ns(&meta) {
            info.state = TransferState::Failed;
            info.error = Some("file changed while preparing".into());
            self.set_transfer(info);
            return Err(Error::Invalid("file changed while it was being read".into()));
        }
        let local_path = Some(path.to_string_lossy().into_owned());
        self.put_offer(OfferRec { id: id.clone(), dev: dev_id.to_string(), name: name.clone(), size: meta.len(), sha256: sha256.clone(), path, mtime_ns: mtime_ns(&meta) });
        info.state = TransferState::Done;
        info.done = info.size;
        self.set_transfer(info);
        self.forget_transfer(&id); // transient: downloads get their own entries
        let fref = FileRef { id, name, size: meta.len(), sha256, dir: Direction::ToPhone, local_path };
        self.add_file_note(dev_id, "pc", fref.clone());
        Ok(fref)
    }

    /// Open an offered file, refusing it if it vanished or changed since it was hashed.
    pub(crate) async fn open_offer(&self, dev_id: &str, id: &str) -> Result<(OfferRec, File), ApiError> {
        let rec = self.offer_for(dev_id, id).ok_or_else(ApiError::not_found)?;
        let meta = tokio::fs::metadata(&rec.path)
            .await
            .map_err(|_| ApiError::new(StatusCode::GONE, "file_gone"))?;
        if meta.len() != rec.size || mtime_ns(&meta) != rec.mtime_ns {
            return Err(ApiError::new(StatusCode::GONE, "file_changed"));
        }
        let file = File::open(&rec.path).await.map_err(|_| ApiError::new(StatusCode::GONE, "file_gone"))?;
        Ok((rec, file))
    }

    /// Body for bytes `start..=end` of an opened offer, reporting progress as it is sent.
    pub(crate) async fn send_body(
        self: &Arc<Self>,
        dev: &Device,
        rec: &OfferRec,
        mut file: File,
        start: u64,
        end_inclusive: u64,
    ) -> Result<Body, ApiError> {
        file.seek(SeekFrom::Start(start)).await.map_err(|e| ApiError::from_io(&e))?;
        let len = end_inclusive + 1 - start;
        let info = TransferInfo {
            id: format!("dl-{}-{}", rec.id, dev.id),
            name: rec.name.clone(),
            size: rec.size,
            done: start,
            dir: Direction::ToPhone,
            state: TransferState::Active,
            device: dev.name.clone(),
            device_id: dev.id.clone(),
            error: None,
        };
        self.set_transfer(info.clone());
        let stream = SendStream {
            inner: ReaderStream::with_capacity(file.take(len), READ_CHUNK),
            shared: self.clone(),
            info,
            start,
            sent: 0,
            expect: len,
            last_emit: Instant::now(),
            finished: false,
        };
        Ok(Body::from_stream(stream))
    }
}

async fn flush(file: &mut File, buf: &mut Vec<u8>) -> std::io::Result<u64> {
    if buf.is_empty() {
        return Ok(0);
    }
    file.write_all(buf).await?;
    file.flush().await?; // tokio files complete writes in the background; wait for them
    let n = buf.len() as u64;
    buf.clear();
    Ok(n)
}

struct SendStream {
    inner: ReaderStream<tokio::io::Take<File>>,
    shared: Arc<Shared>,
    info: TransferInfo,
    start: u64,
    sent: u64,
    expect: u64,
    last_emit: Instant,
    finished: bool,
}

impl Stream for SendStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(b))) => {
                this.sent += b.len() as u64;
                this.info.done = this.start + this.sent;
                if this.sent >= this.expect {
                    // hyper stops polling once Content-Length bytes are out, so the end of the
                    // stream may never be seen: report completion on the last chunk instead.
                    this.finished = true;
                    if this.info.done >= this.info.size {
                        this.info.state = TransferState::Done;
                    }
                    this.shared.set_transfer(this.info.clone());
                } else if this.last_emit.elapsed() >= PROGRESS_EVERY {
                    this.shared.set_transfer(this.info.clone());
                    this.last_emit = Instant::now();
                }
                Poll::Ready(Some(Ok(b)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.finished = true;
                this.info.state = TransferState::Failed;
                this.info.error = Some("read error".into());
                this.shared.set_transfer(this.info.clone());
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                if !this.finished {
                    this.finished = true;
                    this.info.done = this.start + this.sent;
                    if this.info.done >= this.info.size {
                        this.info.state = TransferState::Done;
                    }
                    this.shared.set_transfer(this.info.clone());
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for SendStream {
    fn drop(&mut self) {
        if !self.finished && self.sent < self.expect {
            self.info.done = self.start + self.sent;
            self.info.state = TransferState::Interrupted;
            self.shared.set_transfer(self.info.clone());
        }
    }
}

// ---------------------------------------------------------------- HTTP Range

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RangeSpec {
    /// No usable range: send the whole file with 200.
    Full,
    /// Send bytes `start..=end` with 206.
    Partial(u64, u64),
    /// Answer 416.
    Unsatisfiable,
}

/// Parse a single `Range: bytes=...` header against a file of `size` bytes.
/// Multi-range and malformed headers are ignored (RFC 9110 allows serving the full file).
pub(crate) fn parse_range(header: &str, size: u64) -> RangeSpec {
    let Some(spec) = header.trim().strip_prefix("bytes=") else { return RangeSpec::Full };
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let Some((a, b)) = spec.split_once('-') else { return RangeSpec::Full };
    let (a, b) = (a.trim(), b.trim());
    if size == 0 {
        return RangeSpec::Full;
    }
    if a.is_empty() {
        // suffix: last N bytes
        return match b.parse::<u64>() {
            Ok(0) => RangeSpec::Unsatisfiable,
            Ok(n) => RangeSpec::Partial(size.saturating_sub(n), size - 1),
            Err(_) => RangeSpec::Full,
        };
    }
    let Ok(start) = a.parse::<u64>() else { return RangeSpec::Full };
    if start >= size {
        return RangeSpec::Unsatisfiable;
    }
    if b.is_empty() {
        return RangeSpec::Partial(start, size - 1);
    }
    match b.parse::<u64>() {
        Ok(end) if end >= start => RangeSpec::Partial(start, end.min(size - 1)),
        Ok(_) => RangeSpec::Full,
        Err(_) => RangeSpec::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), RangeSpec::Partial(0, 9));
        assert_eq!(parse_range("bytes=90-", 100), RangeSpec::Partial(90, 99));
        assert_eq!(parse_range("bytes=90-500", 100), RangeSpec::Partial(90, 99));
        assert_eq!(parse_range("bytes=-10", 100), RangeSpec::Partial(90, 99));
        assert_eq!(parse_range("bytes=-500", 100), RangeSpec::Partial(0, 99));
        assert_eq!(parse_range("bytes=100-", 100), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 100), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), RangeSpec::Full);
        assert_eq!(parse_range("bytes=5-2", 100), RangeSpec::Full);
        assert_eq!(parse_range("items=0-1", 100), RangeSpec::Full);
        assert_eq!(parse_range("bytes=abc-", 100), RangeSpec::Full);
        assert_eq!(parse_range("bytes=0-0", 0), RangeSpec::Full);
        assert_eq!(parse_range("bytes=0-99999999999999999999999", 100), RangeSpec::Full);
    }
}
