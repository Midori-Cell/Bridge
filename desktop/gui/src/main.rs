//! `bridge-app`: the native PC window for the bridge.
//!
//! Starts the core (HTTPS server for the phone), then shows the window. A tray icon and a global
//! hotkey keep working while the window is hidden.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod chat;
mod common;
mod hub;
mod icon;
mod icons;
mod minimize;
mod pages;
mod session;
mod shortcut;
mod single;
mod theme;
mod tray;
mod widgets;

use std::path::PathBuf;

use bridge_core::{default_data_dir, default_inbox_dir, Bridge, Config};
use clap::Parser;

#[derive(Parser)]
#[command(name = "bridge-app", version, about = "PC window for the PC <-> phone bridge")]
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
    /// HTTPS port for other PCs (they connect here; needs to be reachable from the network)
    #[arg(long, default_value_t = 8444)]
    peer_port: u16,
    /// Do not announce this PC on the network and do not look for others
    #[arg(long)]
    no_discovery: bool,
    /// Network interface to listen on
    #[arg(long, default_value = "0.0.0.0")]
    bind: std::net::IpAddr,
    /// Start with the window hidden (only when a tray icon is available)
    #[arg(long)]
    hidden: bool,
}

fn fatal(message: &str) -> ! {
    eprintln!("{message}");
    // On Windows the program has no console, so show the problem in a dialog as well.
    rfd::MessageDialog::new().set_title("Bridge").set_description(message).set_level(rfd::MessageLevel::Error).show();
    std::process::exit(1);
}

fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt().with_env_filter("warn").with_target(false).init();

    // A second start only brings the running Bridge to the front (and exits).
    let single = single::claim(single::control_port(args.peer_port));

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| fatal(&format!("Could not start: {e}")));

    let data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    // On the very first start the window asks where received files should go (there is no installer).
    let saved = hub::Settings::load(&data_dir);
    let session = session::Session::detect(saved.keep_in_tray);
    let ask_folder = args.inbox.is_none() && !saved.inbox_confirmed;
    let inbox = args.inbox.or(saved.inbox_dir).unwrap_or_else(default_inbox_dir);
    let mut cfg = Config::new(data_dir, inbox);
    cfg.lan_host = args.lan_host;
    cfg.lan_port = args.lan_port;
    cfg.peer_port = args.peer_port;
    cfg.discovery = !args.no_discovery;
    cfg.bind = args.bind;
    let port = cfg.lan_port;
    let peer_port = cfg.peer_port;
    let bridge = match rt.block_on(Bridge::start(cfg)) {
        Ok(b) => std::sync::Arc::new(b),
        Err(e) => fatal(&format!("Bridge could not start: {e}\n\nIs another copy already running, or are ports {port} and {peer_port} in use?")),
    };

    let hub = hub::Hub::new(bridge, rt.handle().clone());
    hub.window_can_hide.store(session.can_hide, std::sync::atomic::Ordering::SeqCst);
    hub.x11_forced.store(session.force_x11, std::sync::atomic::Ordering::SeqCst);
    hub.spawn_event_task();
    rt.spawn(session::quit_on_signals(hub.clone()));
    if let Some(listener) = single {
        single::serve(listener, hub.clone());
    }

    let icon = egui::IconData { rgba: icon::rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Bridge")
            .with_app_id("bridge")
            .with_inner_size([1040.0, 760.0])
            .with_min_inner_size([720.0, 520.0])
            .with_icon(icon),
        event_loop_builder: session.event_loop_hook(),
        ..Default::default()
    };

    // Debug builds only: `BRIDGE_DEBUG_SCRIPT=show:6,quit:12` calls the same code the tray menu uses,
    // after the given seconds, so hide/show/quit can be tested on a machine without a tray.
    #[cfg(debug_assertions)]
    if let Ok(script) = std::env::var("BRIDGE_DEBUG_SCRIPT") {
        let h = hub.clone();
        std::thread::spawn(move || {
            let mut steps: Vec<(u64, String)> = script
                .split(',')
                .filter_map(|p| p.split_once(':').and_then(|(n, t)| Some((t.parse().ok()?, n.to_string()))))
                .collect();
            steps.sort();
            let started = std::time::Instant::now();
            for (at, what) in steps {
                std::thread::sleep(std::time::Duration::from_secs(at).saturating_sub(started.elapsed()));
                match what.as_str() {
                    "show" => h.show_window(),
                    "quit" => h.quit(),
                    // like a click on the close button (or the system asking the window to close)
                    "close" => h.request_close(),
                    // write a fresh phone pairing code to the file named by BRIDGE_DEBUG_FILE
                    "paircode" => {
                        if let Ok(file) = std::env::var("BRIDGE_DEBUG_FILE") {
                            let _ = std::fs::write(file, h.bridge.new_pairing().code);
                        }
                    }
                    "clip" => h.send_clipboard(),
                    // send the file named by BRIDGE_DEBUG_FILE to the first paired PC
                    "peerfile" => {
                        if let (Some(peer), Ok(file)) = (h.bridge.peers().first().map(|p| p.id.clone()), std::env::var("BRIDGE_DEBUG_FILE")) {
                            h.send_peer_files(&peer, vec![file.into()]);
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    let start_hidden = args.hidden;
    let hub2 = hub.clone();
    let result = eframe::run_native(
        "Bridge",
        options,
        Box::new(move |cc| {
            hub2.set_ctx(cc.egui_ctx.clone());
            {
                let s = hub2.settings();
                theme::set_accent(s.accent, s.custom_accent);
            }
            theme::install(&cc.egui_ctx);
            // egui scrolls 40 points per wheel notch, which feels slow with a mouse wheel on Windows and
            // Linux. macOS reports pixel deltas, so it is not affected and stays as it is.
            if !cfg!(target_os = "macos") {
                cc.egui_ctx.options_mut(|o| o.input_options.line_scroll_speed = 100.0);
            }
            cc.egui_ctx.set_zoom_factor(hub2.settings().zoom.clamp(app::MIN_ZOOM, app::MAX_ZOOM));
            #[cfg(debug_assertions)]
            if std::env::var("BRIDGE_THEME").as_deref() == Ok("light") {
                cc.egui_ctx.set_theme(egui::ThemePreference::Light); // for screenshots; normally follows the system
            }
            let keep = tray::start(&hub2);
            #[cfg(debug_assertions)]
            if std::env::var("BRIDGE_DEBUG_TRAY").is_ok() {
                hub2.tray_ok.store(true, std::sync::atomic::Ordering::SeqCst); // test hiding on a machine without a tray
            }
            if start_hidden && hub2.tray_ok.load(std::sync::atomic::Ordering::SeqCst) {
                hub2.hidden.store(true, std::sync::atomic::Ordering::SeqCst);
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            Ok(Box::new(app::App::new(hub2, keep, ask_folder)))
        }),
    );
    if let Err(e) = result {
        fatal(&format!("The window could not be opened: {e}"));
    }
}
