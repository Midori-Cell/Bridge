//! A PC on the command line for interoperability tests: other programs (the Android app's engine, tested on a
//! plain JVM) drive it over stdin/stdout. One command per line, one answer line per command.
//!
//!   cargo run -p bridge-core --example peer_harness -- <name> <data dir> <inbox dir> [auto]
//!
//! Prints `READY <peer port> <fingerprint>` first. Commands: `pair <host> <port>`, `confirm <id>`, `requests`,
//! `accept <id>`, `peers`, `text <peer> <text>`, `notes <peer>`, `file <peer> <path>`, `transfers`,
//! `acceptfile <id>`, `delete <peer> <mid>`, `unpair <peer>`, `quit`.

use std::io::BufRead;
use std::net::IpAddr;
use std::path::PathBuf;

use bridge_core::*;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: peer_harness <name> <data dir> <inbox dir> [auto]");
        std::process::exit(2);
    }
    let mut cfg = Config::new(PathBuf::from(&args[2]), PathBuf::from(&args[3]));
    cfg.lan_host = Some("127.0.0.1".into());
    cfg.lan_port = 0;
    cfg.peer_port = 0;
    cfg.discovery = false;
    cfg.bind = "127.0.0.1".parse().unwrap();
    let b = std::sync::Arc::new(Bridge::start(cfg).await.expect("start"));
    b.set_pc_name(&args[1]);
    b.set_auto_accept_files(args.get(4).is_some_and(|a| a == "auto"));
    println!("READY {} {}", b.peer_port(), b.fingerprint());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(std::result::Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    while let Some(line) = rx.recv().await {
        let mut it = line.splitn(3, ' ');
        let cmd = it.next().unwrap_or("");
        let a = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("");
        match cmd {
            "pair" => match a.parse::<IpAddr>() {
                Ok(host) => match b.pair_start(host, rest.trim().parse().unwrap_or(0)).await {
                    Ok(p) => println!("PAIR {} {}", p.id, p.code),
                    Err(e) => println!("ERR {e}"),
                },
                Err(_) => println!("ERR bad host"),
            },
            "confirm" => println!("OK {}", b.pair_confirm(a)),
            "outstate" => {
                for p in b.outgoing_pairs() {
                    println!("OUT {} {:?}", p.id, p.state);
                }
                println!("END");
            }
            "requests" => {
                for r in b.pair_requests() {
                    println!("REQ {} {} | {}", r.id, r.code, r.name);
                }
                println!("END");
            }
            "accept" => println!("OK {}", b.pair_accept(a)),
            "peers" => {
                for p in b.peers() {
                    println!("PEER {} {}", p.id, p.name);
                }
                println!("END");
            }
            "text" => match b.send_peer_text(a, rest).await {
                Ok(n) => println!("OK {} {}", n.mid, n.pending),
                Err(e) => println!("ERR {e}"),
            },
            "reply" => {
                let (mid, text) = rest.split_once(' ').unwrap_or((rest, ""));
                match b.send_peer_text_reply(a, text, Some(mid)).await {
                    Ok(n) => println!("OK {} {}", n.mid, n.pending),
                    Err(e) => println!("ERR {e}"),
                }
            }
            "notes" => {
                for n in b.peer_notes(a, 100) {
                    let quote = n.reply.as_ref().map(|r| format!("{}:{}:{}", r.mid, r.src, r.preview)).unwrap_or_default();
                    match (&n.text, &n.file) {
                        (Some(t), _) => println!("NOTE {} {} {} text {} || {}", n.mid, n.src, n.pending, t, quote),
                        (_, Some(f)) => println!("NOTE {} {} {} file {} {}", n.mid, n.src, n.pending, f.name, f.size),
                        _ => {}
                    }
                }
                println!("END");
            }
            "file" => {
                // sending waits for the other user to accept, so it runs on its own; see `transfers`
                let (b2, peer, path) = (b.clone(), a.to_string(), rest.to_string());
                tokio::spawn(async move {
                    let _ = b2.send_peer_file(&peer, path).await;
                });
                println!("OK started");
            }
            "transfers" => {
                for t in b.transfers() {
                    println!("XFER {} {:?} {:?} {} {}/{}", t.id, t.dir, t.state, t.name, t.done, t.size);
                }
                println!("END");
            }
            "acceptfile" => println!("OK {}", b.accept_file(a).await),
            "delete" => println!("OK {}", b.delete_peer_note(a, rest).await),
            "unpair" => println!("OK {}", b.unpair_peer(a)),
            "quit" => break,
            _ => println!("ERR unknown command"),
        }
    }
    if let Ok(b) = std::sync::Arc::try_unwrap(b) {
        b.shutdown().await;
    }
}
