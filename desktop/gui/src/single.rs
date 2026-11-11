//! Only one Bridge per computer. A second start does not open a second window (and does not fail on the busy
//! ports): it asks the running copy to show its window and quits.
//!
//! The running copy listens on 127.0.0.1 (this computer only) on the port after the PC-to-PC port.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::hub::Hub;

const HELLO: &str = "bridge-show";

/// Port for the "show your window" request.
pub fn control_port(peer_port: u16) -> u16 {
    peer_port.wrapping_add(1)
}

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// Ask a running Bridge to show its window. True when one answered.
fn notify_running(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr(port), Duration::from_secs(2)) else { return false };
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(s, "{HELLO}").is_err() {
        return false;
    }
    // The running copy answers "ok"; anything else is some other program on that port.
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).is_ok() && line.trim() == "ok"
}

/// Claim the right to be the running copy. When another Bridge is already running, it is asked to show its
/// window and this process exits. Otherwise the listener to keep (None if the port could not be used, in
/// which case Bridge simply starts without this feature).
pub fn claim(port: u16) -> Option<TcpListener> {
    match TcpListener::bind(addr(port)) {
        Ok(l) => Some(l),
        Err(_) => {
            allow_foreground();
            if notify_running(port) {
                std::process::exit(0);
            }
            None
        }
    }
}

/// Windows only lets the program in front hand the foreground on. This copy was just started by the user (a click,
/// or a file dropped on the icon), so it may pass that right on to the running copy, which then really comes to the front.
#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    // ASFW_ANY: any process may take the foreground
    unsafe {
        AllowSetForegroundWindow(u32::MAX);
    }
}

#[cfg(not(windows))]
fn allow_foreground() {}

/// Answer requests from later starts by showing the window. Runs for the life of the program.
pub fn serve(listener: TcpListener, hub: Arc<Hub>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_ok() && line.trim() == HELLO {
                hub.show_window();
                let _ = (&stream).write_all(b"ok\n");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_reaches_the_first() {
        let first = claim(0).expect("free port");
        let port = first.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (stream, _) = first.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(b"ok\n").unwrap();
            line
        });
        assert!(notify_running(port));
        assert_eq!(seen.join().unwrap().trim(), HELLO);
    }

    #[test]
    fn something_else_on_the_port_is_not_a_running_copy() {
        let other = TcpListener::bind(addr(0)).unwrap();
        let port = other.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((s, _)) = other.accept() {
                let _ = (&s).write_all(b"HTTP/1.1 400\n");
            }
        });
        assert!(!notify_running(port));
    }
}
