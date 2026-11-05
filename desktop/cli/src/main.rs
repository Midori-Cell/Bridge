//! `bridge`: headless PC agent. Step-1 test tool; the native window replaces the console later.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bridge_core::{
    default_data_dir, default_inbox_dir, Bridge, Config, Direction, Event, NoteKind, TransferInfo, TransferState,
};
use clap::Parser;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast::error::RecvError;

#[derive(Parser)]
#[command(name = "bridge", version, about = "PC agent for the PC <-> phone bridge")]
struct Args {
    /// Where keys, paired phones, history and half-received files are kept
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Folder for files received from the phone
    #[arg(long)]
    inbox: Option<PathBuf>,
    /// Address the phone should use (default: detected LAN address)
    #[arg(long)]
    lan_host: Option<String>,
    /// HTTPS port for the phone
    #[arg(long, default_value_t = 8443)]
    lan_port: u16,
    /// Network interface to listen on
    #[arg(long, default_value = "0.0.0.0")]
    bind: std::net::IpAddr,
    /// Draw the QR code for a light terminal background (default assumes dark)
    #[arg(long)]
    light_terminal: bool,
    /// Show debug logging
    #[arg(short, long)]
    verbose: bool,
}

const HELP: &str = "\
Commands:
  pair                 show a QR code to pair a phone
  devices              list paired phones
  unpair <number>      unpair a phone (number from 'devices')
  text <message>       send a text note to the phone
  send <path>          offer a file to the phone
  history [n]          show the last n notes (default 10)
  clear                clear the note history
  inbox                show where received files are saved
  help                 this list
  quit                 stop the agent";

fn size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

fn print_qr(url: &str, light_terminal: bool) {
    use qrcode::render::unicode::Dense1x2;
    match qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::M) {
        Ok(code) => {
            let mut r = code.render::<Dense1x2>();
            r.quiet_zone(true);
            if !light_terminal {
                // a dark terminal shows "empty" as dark, so swap polarity to keep the code scannable
                r.dark_color(Dense1x2::Light).light_color(Dense1x2::Dark);
            }
            println!("{}", r.build());
        }
        Err(e) => println!("(could not draw QR code: {e})"),
    }
}

/// Print transfer progress at 25% steps and on every state change, not on every event.
fn report_transfer(t: &TransferInfo, seen: &mut HashMap<String, (TransferState, u64)>) {
    let bucket = if t.size == 0 { 4 } else { t.done * 4 / t.size };
    let prev = seen.get(&t.id).copied();
    if prev == Some((t.state, bucket)) {
        return;
    }
    seen.insert(t.id.clone(), (t.state, bucket));
    let arrow = match t.dir {
        Direction::ToPc => format!("<- {}", t.device),
        Direction::ToPhone => format!("-> {}", if t.device.is_empty() { "phone" } else { &t.device }),
        Direction::FromPeer => format!("<- PC {}", t.device),
        Direction::ToPeer => format!("-> PC {}", t.device),
    };
    let what = format!("{} ({})", t.name, size(t.size));
    // The transient "preparing" entry of an offered file ends silently; 'ready for the phone' follows.
    if t.dir == Direction::ToPhone && t.device.is_empty() && t.state == TransferState::Done {
        return;
    }
    match t.state {
        TransferState::Hashing => println!("[file] preparing {what}..."),
        TransferState::Active if t.done == 0 => println!("[file] {arrow}: {what} started"),
        TransferState::Active => println!("[file] {arrow}: {what} {}%", bucket * 25),
        TransferState::Done => println!("[file] {arrow}: {what} done"),
        TransferState::Interrupted => println!("[file] {arrow}: {what} interrupted at {} (the phone can resume)", size(t.done)),
        TransferState::Failed => println!("[file] {arrow}: {what} FAILED: {}", t.error.as_deref().unwrap_or("unknown")),
        TransferState::Offered => println!("[file] {arrow}: {what} offered"),
        TransferState::Declined => println!("[file] {arrow}: {what} declined"),
        TransferState::Cancelled => println!("[file] {arrow}: {what} cancelled"),
    }
}

async fn print_events(bridge: Arc<Bridge>) {
    let mut rx = bridge.events();
    let mut seen = HashMap::new();
    loop {
        match rx.recv().await {
            Ok(Event::NoteAdded(n)) if n.src != "pc" => match (n.kind, n.text, n.file) {
                (NoteKind::Text, Some(text), _) => println!("[{}] {text}", n.src),
                (NoteKind::File, _, Some(f)) => println!("[{}] sent file: {} ({}) saved in the inbox", n.src, f.name, size(f.size)),
                _ => {}
            },
            Ok(Event::DevicesChanged) => println!("[devices] {} paired", bridge.devices().len()),
            Ok(Event::Transfer(t)) => {
                // The command line has nobody to ask: files that are offered to it are accepted.
                if t.state == TransferState::Offered && matches!(t.dir, Direction::ToPc | Direction::FromPeer) {
                    println!("[file] accepting {} from {}", t.name, t.device);
                    bridge.accept_file(&t.id).await;
                }
                report_transfer(&t, &mut seen)
            }
            Ok(_) => {}
            Err(RecvError::Lagged(n)) => println!("(missed {n} events)"),
            Err(RecvError::Closed) => return,
        }
    }
}

async fn run_command(bridge: &Arc<Bridge>, line: &str, light: bool) -> bool {
    let (cmd, rest) = line.split_once(char::is_whitespace).map(|(c, r)| (c, r.trim())).unwrap_or((line, ""));
    match cmd {
        "" => {}
        "help" | "?" => println!("{HELP}"),
        "quit" | "exit" => return false,
        "pair" => {
            let p = bridge.new_pairing();
            print_qr(&p.url, light);
            println!("Scan with the phone camera, then accept the certificate warning once.");
            println!("Or open: {}", p.url);
            println!("Code {} (works once, expires in {} min)", p.code, p.expires_in.as_secs() / 60);
        }
        "devices" => {
            let devs = bridge.devices();
            if devs.is_empty() {
                println!("No phones paired. Type 'pair'.");
            }
            for (i, d) in devs.iter().enumerate() {
                let seen = d.last_seen.map_or("not seen since start".to_string(), |_| "seen".to_string());
                println!("  {}. {} ({seen})", i + 1, d.name);
            }
        }
        "unpair" => match rest.parse::<usize>().ok().and_then(|n| bridge.devices().into_iter().nth(n.wrapping_sub(1))) {
            Some(d) => {
                bridge.unpair(&d.id);
                println!("Unpaired {}.", d.name);
            }
            None => println!("Usage: unpair <number>  (see 'devices')"),
        },
        // The console has no chat window, so it writes to each paired phone's own conversation.
        "text" => {
            let phones = bridge.devices();
            if phones.is_empty() {
                println!("No phone is paired.");
            }
            for d in phones {
                match bridge.send_text(&d.id, rest) {
                    Ok(_) => println!("Sent to {}.", d.name),
                    Err(e) => println!("Not sent to {}: {e:?}", d.name),
                }
            }
        }
        "send" => {
            let path = PathBuf::from(rest.trim_matches(|c| c == '"' || c == '\''));
            if rest.is_empty() {
                println!("Usage: send <path>");
            } else {
                for d in bridge.devices() {
                    let (b, path) = (bridge.clone(), path.clone());
                    tokio::spawn(async move {
                        match b.send_file(&d.id, &path).await {
                            Ok(f) => println!("[file] ready for {}: {} ({})", d.name, f.name, size(f.size)),
                            Err(e) => println!("[file] could not offer {} to {}: {e}", path.display(), d.name),
                        }
                    });
                }
            }
        }
        "history" => {
            let n = rest.parse::<usize>().unwrap_or(10);
            for d in bridge.devices() {
                println!("{}:", d.name);
                for note in bridge.notes(&d.id, n) {
                    match (note.text, note.file) {
                        (Some(t), _) => println!("  [{}] {t}", note.src),
                        (_, Some(f)) => println!("  [{}] file {} ({})", note.src, f.name, size(f.size)),
                        _ => {}
                    }
                }
            }
        }
        "clear" => {
            for d in bridge.devices() {
                bridge.clear_history(&d.id);
            }
            println!("History cleared.");
        }
        "inbox" => println!("{}", bridge.inbox_dir().display()),
        other => println!("Unknown command '{other}'. Type 'help'."),
    }
    true
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let filter = if args.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).init();

    let mut cfg = Config::new(args.data_dir.unwrap_or_else(default_data_dir), args.inbox.unwrap_or_else(default_inbox_dir));
    cfg.lan_host = args.lan_host;
    cfg.lan_port = args.lan_port;
    cfg.bind = args.bind;

    let bridge = match Bridge::start(cfg).await {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("Could not start: {e}\nIs another copy running, or is port {} in use?", args.lan_port);
            std::process::exit(1);
        }
    };

    println!("Bridge {}", env!("CARGO_PKG_VERSION"));
    println!("  Phone address: https://{}:{}/", bridge.lan_host(), bridge.lan_port());
    println!("  Fingerprint  : {}", bridge.fingerprint());
    println!("  Inbox        : {}", bridge.inbox_dir().display());
    println!("  Data folder  : {}", bridge.data_dir().display());
    println!("Type 'pair' to connect a phone, 'help' for all commands.\n");

    tokio::spawn(print_events(bridge.clone()));

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) => if !run_command(&bridge, l.trim(), args.light_terminal).await { break },
                _ => {
                    // stdin closed (e.g. run as a service): keep serving until Ctrl+C
                    let _ = tokio::signal::ctrl_c().await;
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    println!("Stopping.");
}
