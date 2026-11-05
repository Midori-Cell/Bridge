//! Public data types: configuration, notes, devices, transfers, events.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- config

#[derive(Clone, Debug)]
pub struct Config {
    /// Keys, paired devices, note history, half-received files.
    pub data_dir: PathBuf,
    /// Where finished files from the phone are saved.
    pub inbox_dir: PathBuf,
    /// Address shown in the pairing QR code. Detected automatically when `None`.
    pub lan_host: Option<String>,
    /// HTTPS port for the phone. 0 picks a free one (used by tests).
    pub lan_port: u16,
    pub bind: IpAddr,
    pub max_file_size: u64,
    pub code_ttl: Duration,
    pub max_pair_fails: u32,
    pub lockout: Duration,
    /// A sender that goes silent for this long mid-upload is dropped (progress is kept).
    pub idle_timeout: Duration,
    /// HTTPS port for other PCs (mutual TLS). 0 picks a free one (used by tests).
    pub peer_port: u16,
    /// Announce this PC on the local network and look for other PCs.
    pub discovery: bool,
}

impl Config {
    pub fn new(data_dir: PathBuf, inbox_dir: PathBuf) -> Self {
        Self {
            data_dir,
            inbox_dir,
            lan_host: None,
            lan_port: 8443,
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            max_file_size: 100 * 1024 * 1024 * 1024,
            code_ttl: Duration::from_secs(300),
            max_pair_fails: 5,
            lockout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(30),
            peer_port: 8444,
            discovery: true,
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("pcphone-bridge")
}

pub fn default_inbox_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Bridge")
}

// ---------------------------------------------------------------- notes

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ToPc,
    ToPhone,
    /// To another PC.
    ToPeer,
    /// From another PC.
    FromPeer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Text,
    File,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileRef {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub dir: Direction,
    /// Where the file is on this PC: the inbox copy for received files, the original for sent ones.
    /// Never sent to a phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

/// What a message answers: a short quote, taken from this PC's own copy of the original.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub mid: String,
    /// Who wrote the original ("pc" or a device name).
    pub src: String,
    pub preview: String,
}

/// One entry in the shared history: a text note or a file event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    /// Message id, the same on every device that has this message (used for replies and deletes).
    /// For files it is the file id.
    #[serde(default)]
    pub mid: String,
    pub ts: f64,
    /// "pc" or the name of the phone that sent it.
    pub src: String,
    /// The phone whose conversation this message belongs to (a PC-to-PC message has none). Every phone
    /// has its own conversation with this PC; a phone never sees the messages of another phone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dev: String,
    pub kind: NoteKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
    /// Cleared from the chat on this PC. The note stays in the list "Notes and files" (and on the phone).
    #[serde(default, skip_serializing_if = "is_false")]
    pub chat_hidden: bool,
    /// Cleared from the list "Notes and files" on this PC. The note stays in the chat (and on the phone).
    #[serde(default, skip_serializing_if = "is_false")]
    pub list_hidden: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Which view of a conversation "Clear" empties. The two views are independent: a message that is cleared
/// from one stays in the other, and is deleted from this PC only when it has been cleared from both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clear {
    Chat,
    List,
    Both,
}

impl Clear {
    /// Mark every note picked by `mine` as cleared from the chosen view(s), take out the ones that are
    /// now cleared from both, and return those.
    pub(crate) fn apply(self, notes: &mut Vec<Note>, mine: impl Fn(&Note) -> bool) -> Vec<Note> {
        for n in notes.iter_mut().filter(|n| mine(n)) {
            n.chat_hidden |= self != Clear::List;
            n.list_hidden |= self != Clear::Chat;
        }
        let (gone, keep): (Vec<Note>, Vec<Note>) = std::mem::take(notes).into_iter().partition(|n| n.chat_hidden && n.list_hidden);
        *notes = keep;
        gone
    }
}

impl Note {
    /// The note as a phone may see it: without paths from this PC.
    pub fn for_phone(&self) -> Note {
        let mut n = self.clone();
        n.dev.clear();
        n.chat_hidden = false;
        n.list_hidden = false;
        if let Some(f) = n.file.as_mut() {
            f.local_path = None;
        }
        n
    }
}

// ---------------------------------------------------------------- devices

/// A paired phone as shown to the user (never contains the token).
#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created: f64,
    pub last_seen: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DeviceRec {
    pub name: String,
    pub token_hash: String,
    pub created: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OfferRec {
    pub id: String,
    /// The phone this file was offered to; no other phone may fetch it.
    #[serde(default)]
    pub dev: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub path: PathBuf,
    /// Modification time when the hash was taken; a changed file is refused.
    pub mtime_ns: u128,
}

/// Everything saved to `state.json`.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Persist {
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceRec>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub offers: BTreeMap<String, OfferRec>,
    /// Trusted other PCs, keyed by the SHA-256 fingerprint of their certificate.
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRec>,
    /// One conversation per trusted PC. Never mixed with the phone notes.
    #[serde(default)]
    pub peer_notes: BTreeMap<String, Vec<Note>>,
    /// Name shown to other PCs. Defaults to the computer's host name.
    #[serde(default)]
    pub pc_name: Option<String>,
    /// Do not announce this PC on the network.
    #[serde(default)]
    pub hidden: bool,
    /// Receive files without asking. Off by default: the user accepts each incoming file.
    #[serde(default)]
    pub auto_accept_files: bool,
    /// A file this PC offers to another PC is withdrawn when nobody accepts it within this many seconds.
    /// 0: keep waiting (the other PC drops an unanswered offer after 15 minutes anyway).
    #[serde(default)]
    pub offer_timeout_secs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PeerRec {
    pub name: String,
    pub created: f64,
    /// Last address that worked, `ip:port`. Used when discovery has not found the PC.
    #[serde(default)]
    pub last_addr: Option<String>,
}

// ---------------------------------------------------------------- other PCs

/// A trusted (paired) PC.
#[derive(Clone, Debug)]
pub struct Peer {
    /// SHA-256 fingerprint of its certificate, lowercase hex. This *is* its identity.
    pub id: String,
    pub name: String,
    pub created: f64,
    /// Seen on the network or answered recently.
    pub online: bool,
}

/// A PC that announced itself on the local network.
#[derive(Clone, Debug)]
pub struct Nearby {
    pub id: String,
    pub name: String,
    pub addrs: Vec<std::net::IpAddr>,
    pub port: u16,
    pub paired: bool,
}

/// Another PC asks to pair. Both screens must show the same `code` before this is accepted.
#[derive(Clone, Debug)]
pub struct IncomingPair {
    pub id: String,
    pub name: String,
    /// Six digits, formatted "123 456".
    pub code: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutPairState {
    /// Waiting for the other PC's user (and for yours to confirm the code).
    Waiting,
    /// The other PC accepted; confirm the code on this PC to finish.
    PeerAccepted,
    Done,
    Declined,
    Failed,
}

/// Pairing that this PC started.
#[derive(Clone, Debug)]
pub struct OutgoingPair {
    pub id: String,
    pub peer_name: String,
    pub code: String,
    pub state: OutPairState,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- pairing

#[derive(Clone, Debug)]
pub struct PairingInfo {
    pub code: String,
    /// `https://HOST:PORT/#pair=CODE&fp=FINGERPRINT`, the content of the QR code.
    pub url: String,
    pub expires_in: Duration,
}

// ---------------------------------------------------------------- transfers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    /// Preparing a file for the phone (computing its checksum).
    Hashing,
    /// Incoming: the other side wants to send, waiting for this PC's user. Outgoing: waiting for the other side to accept.
    Offered,
    Active,
    Done,
    /// Connection lost; the phone can resume.
    Interrupted,
    Failed,
    /// The receiver said no.
    Declined,
    /// Stopped by either side. `error` says by whom.
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
pub struct TransferInfo {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub dir: Direction,
    pub state: TransferState,
    pub device: String,
    /// Phone id or the fingerprint of the other PC. Empty for files offered to phones.
    pub device_id: String,
    /// Reason for `Failed`; who stopped it for `Declined` and `Cancelled`.
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// A message was deleted. `peer_id` is `None` for the phone conversation.
    NoteRemoved { peer_id: Option<String>, mid: String },
    NoteAdded(Note),
    DevicesChanged,
    /// Paired PCs, or PCs found on the network, changed.
    PeersChanged,
    /// A note arrived from, or was sent to, a paired PC.
    PeerNote { peer_id: String, note: Note },
    /// Another PC wants to pair; show the code to the user.
    PairRequest(IncomingPair),
    /// Progress of a pairing this PC started.
    PairUpdate(OutgoingPair),
    /// Any change to a transfer: start, progress (throttled), finish, failure.
    Transfer(TransferInfo),
}
