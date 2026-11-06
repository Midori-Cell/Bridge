//! bridge-core: everything the PC side needs, with no user interface.
//!
//! ```no_run
//! # async fn demo() -> bridge_core::Result<()> {
//! use bridge_core::{Bridge, Config, default_data_dir, default_inbox_dir};
//! let bridge = Bridge::start(Config::new(default_data_dir(), default_inbox_dir())).await?;
//! let pairing = bridge.new_pairing();          // show pairing.url as a QR code
//! let mut events = bridge.events();            // notes, devices, transfer progress
//! # Ok(()) }
//! ```

mod discovery;
mod error;
mod identity;
mod model;
mod peer;
mod server;
mod state;
mod transfer;
mod util;

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub use error::{Error, Result};
pub use identity::{fingerprint_of, peer_client_config_from_dir, pinned_client_config};
pub use peer::PeerError;
pub use model::*;
pub use state::{NoteError, MAX_TEXT_CHARS};
pub use util::sanitize_name;

use state::Shared;

/// A running PC agent: the HTTPS server for the phone plus all state behind it.
pub struct Bridge {
    shared: Arc<Shared>,
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Best guess at this machine's LAN address. No packet is actually sent.
pub fn detect_lan_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("10.255.255.255:1")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

impl Bridge {
    /// Create or load the identity, bind the phone port and start serving.
    pub async fn start(cfg: Config) -> Result<Bridge> {
        let lan_host = cfg.lan_host.clone().unwrap_or_else(detect_lan_ip);
        std::fs::create_dir_all(&cfg.data_dir)?;
        let identity = Arc::new(identity::load_or_create(&cfg.data_dir, &lan_host)?);
        let phone_tls = identity::server_config(&identity)?;
        let peer_tls = identity::peer_server_config(&identity)?;
        let discovery_on = cfg.discovery;
        let phone_listener = TcpListener::bind((cfg.bind, cfg.lan_port)).await?;
        let peer_listener = TcpListener::bind((cfg.bind, cfg.peer_port)).await?;
        let phone_port = phone_listener.local_addr()?.port();
        let peer_port = peer_listener.local_addr()?.port();
        let shared = Shared::new(cfg, identity, lan_host, phone_port, peer_port)?;
        shared.load_incoming();
        let cancel = CancellationToken::new();
        let tasks = vec![
            tokio::spawn(server::serve(server::router(shared.clone()), phone_listener, phone_tls, cancel.clone())),
            tokio::spawn(server::serve(peer::router(shared.clone()), peer_listener, peer_tls, cancel.clone())),
            tokio::spawn(peer::outbox_loop(shared.clone(), cancel.clone())),
        ];
        if discovery_on {
            match discovery::Discovery::start(&shared) {
                Ok(d) => {
                    d.refresh(&shared.pc_name(), &shared.fingerprint, peer_port, shared.is_hidden());
                    *shared.peer.discovery.lock().unwrap_or_else(|e| e.into_inner()) = Some(d);
                }
                // not fatal: pairing by address still works
                Err(e) => tracing::warn!("network discovery unavailable: {e}"),
            }
        }
        Ok(Bridge { shared, cancel, tasks })
    }

    /// Stop accepting connections and wait for the server tasks to end.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        if let Some(d) = self.shared.peer.discovery.lock().unwrap_or_else(|e| e.into_inner()).take() {
            d.shutdown();
        }
        for t in self.tasks {
            let _ = t.await;
        }
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.shared.subscribe()
    }

    pub fn lan_host(&self) -> &str {
        &self.shared.lan_host
    }

    pub fn lan_port(&self) -> u16 {
        self.shared.lan_port
    }

    /// Port other PCs connect to (mutual TLS).
    pub fn peer_port(&self) -> u16 {
        self.shared.peer_port
    }

    /// SHA-256 fingerprint of the PC certificate, lowercase hex.
    pub fn fingerprint(&self) -> &str {
        &self.shared.fingerprint
    }

    /// The folder received files are saved in.
    pub fn inbox_dir(&self) -> PathBuf {
        self.shared.inbox()
    }

    /// Save received files in another folder from now on (it is created when the first file arrives).
    /// Files already received stay where they are.
    pub fn set_inbox_dir(&self, dir: PathBuf) {
        self.shared.set_inbox(dir);
    }

    pub fn data_dir(&self) -> &Path {
        &self.shared.cfg.data_dir
    }

    /// A fresh one-time pairing code and the URL to show as a QR code.
    pub fn new_pairing(&self) -> PairingInfo {
        let s = &self.shared;
        let code = s.new_pair_code();
        let host = if s.lan_host.contains(':') { format!("[{}]", s.lan_host) } else { s.lan_host.clone() };
        // Everything after '#' stays on the phone; it is never sent to the server.
        let url = format!("https://{host}:{}/#pair={code}&fp={}", s.lan_port, s.fingerprint);
        PairingInfo { code, url, expires_in: s.cfg.code_ttl }
    }

    pub fn devices(&self) -> Vec<Device> {
        self.shared.devices()
    }

    /// Unpair a phone. It is refused immediately and its half-received files are deleted.
    pub fn unpair(&self, device_id: &str) -> bool {
        self.shared.revoke(device_id)
    }

    /// Send a text note to one paired phone. Every phone has its own conversation; no other phone sees it.
    pub fn send_text(&self, device_id: &str, text: &str) -> std::result::Result<Note, NoteError> {
        self.shared.add_text_note(device_id, "pc", text, None)
    }

    /// Like `send_text`, as an answer to the message with id `reply_to` (from the same conversation).
    pub fn send_text_reply(&self, device_id: &str, text: &str, reply_to: Option<&str>) -> std::result::Result<Note, NoteError> {
        self.shared.add_text_note(device_id, "pc", text, reply_to)
    }

    /// Delete a message of the conversation with a phone (by its `mid`). The phone drops it too.
    /// A file received from the phone is deleted from the inbox; a file offered to it is withdrawn.
    pub fn delete_note(&self, device_id: &str, mid: &str) -> bool {
        self.shared.delete_note(device_id, mid)
    }

    /// Offer a local file to one phone. Computes its checksum first (can take a while for big files).
    pub async fn send_file(&self, device_id: &str, path: impl AsRef<Path>) -> Result<FileRef> {
        self.shared.make_offer(device_id, path.as_ref()).await
    }

    /// Most recent notes of the conversation with one phone, oldest first.
    pub fn notes(&self, device_id: &str, limit: usize) -> Vec<Note> {
        self.shared.notes_since(device_id, None, limit)
    }

    /// Delete the whole conversation with one phone from this PC, so the phone loses it too (it reads its history
    /// from here); the other phones keep theirs. The window's Clear buttons only hide (see `clear_chat`).
    pub fn clear_history(&self, device_id: &str) {
        self.shared.clear_notes(device_id, Clear::Both, true);
    }

    /// Hide the chat with one phone on this PC. The phone keeps its history and the list of files keeps its entries.
    pub fn clear_chat(&self, device_id: &str) {
        self.shared.clear_notes(device_id, Clear::Chat, false);
    }

    /// Hide the list of files of one phone on this PC. The chat keeps its messages; the phone is not affected.
    pub fn clear_list(&self, device_id: &str) {
        self.shared.clear_notes(device_id, Clear::List, false);
    }

    pub fn transfers(&self) -> Vec<TransferInfo> {
        self.shared.transfers()
    }

    /// Whether files are received without asking (default: off, the user accepts each one).
    pub fn auto_accept_files(&self) -> bool {
        self.shared.auto_accept_files()
    }

    pub fn set_auto_accept_files(&self, on: bool) {
        self.shared.set_auto_accept_files(on);
    }

    /// After how many seconds a file offered to another PC is withdrawn when nobody accepts it (0: never).
    pub fn offer_timeout_secs(&self) -> u32 {
        self.shared.offer_timeout_secs()
    }

    pub fn set_offer_timeout_secs(&self, secs: u32) {
        self.shared.set_offer_timeout_secs(secs);
    }

    /// Accept a file another device wants to send (a transfer in state `Offered`).
    pub async fn accept_file(&self, transfer_id: &str) -> bool {
        self.shared.accept_incoming(transfer_id).await.is_ok()
    }

    /// Say no to an offered file.
    pub fn decline_file(&self, transfer_id: &str) -> bool {
        self.shared.decline_incoming(transfer_id)
    }

    /// Stop a transfer: a file being received, or one this PC is sending (also while it waits to be accepted).
    pub fn cancel_transfer(&self, transfer_id: &str) -> bool {
        self.shared.cancel_receiving(transfer_id) || self.shared.cancel_sending(transfer_id)
    }

    /// Take a finished, declined or cancelled transfer off the list. For a transfer with another PC,
    /// that PC's list is cleaned up too.
    pub async fn dismiss_transfer(&self, transfer_id: &str) -> bool {
        let peer = self.shared.transfers().into_iter().find(|t| t.id == transfer_id).map(|t| (t.dir, t.device_id));
        match peer {
            Some((Direction::ToPeer | Direction::FromPeer, fp)) if !fp.is_empty() => self.shared.dismiss_peer_transfer(&fp, transfer_id).await,
            _ => self.shared.dismiss_transfer(transfer_id, None),
        }
    }

    /// Change the name of a paired phone.
    pub fn rename_device(&self, device_id: &str, name: &str) -> bool {
        self.shared.rename_device(device_id, name)
    }

    // ------------------------------------------------------------ other PCs

    /// Name this PC shows to other PCs (default: the computer's host name).
    pub fn pc_name(&self) -> String {
        self.shared.pc_name()
    }

    pub fn set_pc_name(&self, name: &str) {
        self.shared.set_pc_name(name);
    }

    /// Whether this PC announces itself to other PCs on the network.
    pub fn is_discoverable(&self) -> bool {
        !self.shared.is_hidden()
    }

    pub fn set_discoverable(&self, on: bool) {
        self.shared.set_hidden(!on);
    }

    /// PCs found on the local network (paired or not).
    pub fn nearby(&self) -> Vec<Nearby> {
        self.shared.nearby()
    }

    /// Paired PCs.
    pub fn peers(&self) -> Vec<Peer> {
        self.shared.peers()
    }

    /// Start pairing with the PC at `host:port` (its PC port). Show `code` on screen; the other PC shows the same one.
    pub async fn pair_start(&self, host: std::net::IpAddr, port: u16) -> std::result::Result<OutgoingPair, PeerError> {
        self.shared.pair_start(host, port).await
    }

    /// You compared the codes and they match.
    pub fn pair_confirm(&self, pair_id: &str) -> bool {
        self.shared.pair_confirm(pair_id)
    }

    /// Abort a pairing this PC started (also withdraws it on the other PC).
    pub fn pair_cancel(&self, pair_id: &str) {
        self.shared.pair_cancel_out(pair_id);
    }

    pub fn outgoing_pairs(&self) -> Vec<OutgoingPair> {
        self.shared.out_pairs()
    }

    /// Pairing requests from other PCs that wait for a decision.
    pub fn pair_requests(&self) -> Vec<IncomingPair> {
        self.shared.pair_requests()
    }

    /// Requests you accepted that wait until the other PC's user confirms too.
    pub fn pair_waiting(&self) -> Vec<IncomingPair> {
        self.shared.pair_waiting()
    }

    /// You compared the codes on both screens and they match.
    pub fn pair_accept(&self, request_id: &str) -> bool {
        self.shared.accept_pair(request_id)
    }

    pub fn pair_decline(&self, request_id: &str) -> bool {
        self.shared.decline_pair(request_id)
    }

    /// Remove a paired PC, its conversation, and tell it (if reachable).
    pub fn unpair_peer(&self, peer_id: &str) -> bool {
        self.shared.unpair_peer(peer_id)
    }

    pub async fn send_peer_text(&self, peer_id: &str, text: &str) -> std::result::Result<Note, PeerError> {
        self.shared.send_peer_text(peer_id, text, None).await
    }

    /// Like `send_peer_text`, as an answer to the message with id `reply_to`.
    pub async fn send_peer_text_reply(&self, peer_id: &str, text: &str, reply_to: Option<&str>) -> std::result::Result<Note, PeerError> {
        self.shared.send_peer_text(peer_id, text, reply_to).await
    }

    /// Delete a message in the conversation with another PC, here and (if it is reachable) there.
    /// A received file is deleted from the inbox here; on the other PC only the message goes.
    pub async fn delete_peer_note(&self, peer_id: &str, mid: &str) -> bool {
        self.shared.delete_peer_note(peer_id, mid).await
    }

    pub async fn send_peer_file(&self, peer_id: &str, path: impl AsRef<Path>) -> std::result::Result<FileRef, PeerError> {
        self.shared.send_peer_file(peer_id, path.as_ref()).await
    }

    /// The conversation with one paired PC, oldest first.
    pub fn peer_notes(&self, peer_id: &str, limit: usize) -> Vec<Note> {
        self.shared.peer_notes(peer_id, limit)
    }

    /// Clear the conversation with one paired PC completely (chat and list).
    pub fn clear_peer_notes(&self, peer_id: &str) {
        self.shared.clear_peer_notes(peer_id, Clear::Both);
    }

    pub fn clear_peer_chat(&self, peer_id: &str) {
        self.shared.clear_peer_notes(peer_id, Clear::Chat);
    }

    pub fn clear_peer_list(&self, peer_id: &str) {
        self.shared.clear_peer_notes(peer_id, Clear::List);
    }
}

/// Where the data folder lives if the caller does not choose one.
pub fn data_dir_or_default(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(default_data_dir)
}
