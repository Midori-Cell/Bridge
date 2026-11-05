//! What the window system lets the program do, and how the program learns that the system wants it to end.
//!
//! Linux: a native Wayland window cannot be hidden (the protocol has no such request), so "keep running in
//! the tray when the window is closed" would leave the window open, and a window that refuses to close also
//! keeps the desktop from logging out or shutting down. When that option is on, a Wayland session therefore
//! starts the window through XWayland, where hiding works. Everywhere else hiding works as it is.

use std::sync::Arc;

use crate::hub::Hub;

pub struct Session {
    /// Closing the window can hide it.
    pub can_hide: bool,
    /// The window is started through X11 although the session is Wayland.
    pub force_x11: bool,
}

impl Session {
    pub fn detect(keep_in_tray: bool) -> Session {
        #[cfg(target_os = "linux")]
        {
            let set = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
            if set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET") {
                // Needs XWayland (DISPLAY). Without it the window stays native and closing quits.
                let force = keep_in_tray && set("DISPLAY");
                return Session { can_hide: force, force_x11: force };
            }
        }
        let _ = keep_in_tray;
        Session { can_hide: true, force_x11: false }
    }

    /// Makes the window system start with X11 when `force_x11` is set.
    pub fn event_loop_hook(&self) -> Option<eframe::EventLoopBuilderHook> {
        #[cfg(target_os = "linux")]
        if self.force_x11 {
            return Some(Box::new(|builder| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }));
        }
        None
    }
}

/// Wait for the system to ask the program to end (logout, shutdown, closed terminal) and quit then.
#[cfg(unix)]
pub async fn quit_on_signals(hub: Arc<Hub>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup())) else {
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
        _ = hup.recv() => {}
    }
    hub.quit_quietly();
}

#[cfg(not(unix))]
pub async fn quit_on_signals(_hub: Arc<Hub>) {}
