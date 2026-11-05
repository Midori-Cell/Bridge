//! macOS: minimizing with `ViewportCommand::Minimized(true)` made the window freeze.
//!
//! egui notes "minimized" when it sends the command, but on a Mac it never asks the system again (that
//! can deadlock), so after the window is restored from the Dock egui still believes it is minimized and
//! stops drawing it. The yellow button is not affected because egui never sees it.
//!
//! The fix: remember that the program minimized the window itself; once the window has the keyboard focus
//! again, it has been restored, so tell egui `Minimized(false)`. The system ignores that (the window is not
//! minimized) and egui drops its stale note.

use std::time::{Duration, Instant};

/// A minimized window cannot have the focus, but while it is still shrinking into the Dock it might.
const SETTLE: Duration = Duration::from_millis(1200);

#[derive(Default)]
pub struct Watch {
    since: Option<Instant>,
}

impl Watch {
    /// Call when the program sends `Minimized(true)`.
    pub fn minimized_by_us(&mut self, now: Instant) {
        self.since = Some(now);
    }

    /// Still waiting for the window to come back?
    pub fn waiting(&self) -> bool {
        self.since.is_some()
    }

    /// Call on every tick of `App::logic` (it also runs while the window is minimized) with what egui
    /// currently believes. True when the program should now send `Minimized(false)`.
    pub fn restored(&mut self, now: Instant, egui_thinks_minimized: Option<bool>, focused: Option<bool>) -> bool {
        let Some(since) = self.since else { return false };
        if egui_thinks_minimized == Some(false) {
            self.since = None; // restored by other means (the tray icon already told egui)
            return false;
        }
        if focused == Some(true) && now.duration_since(since) > SETTLE {
            self.since = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_nothing_when_the_program_did_not_minimize() {
        let mut w = Watch::default();
        assert!(!w.restored(Instant::now(), Some(true), Some(true)));
    }

    #[test]
    fn waits_while_the_window_is_in_the_dock_and_acts_once_when_focus_returns() {
        let t = Instant::now();
        let mut w = Watch::default();
        w.minimized_by_us(t);
        assert!(!w.restored(t + Duration::from_millis(100), Some(true), Some(true)), "still shrinking");
        assert!(!w.restored(t + Duration::from_secs(5), Some(true), Some(false)), "in the Dock");
        assert!(w.waiting());
        assert!(w.restored(t + Duration::from_secs(9), Some(true), Some(true)), "restored from the Dock");
        assert!(!w.waiting());
        assert!(!w.restored(t + Duration::from_secs(10), Some(true), Some(true)), "only once");
    }

    #[test]
    fn stops_waiting_when_egui_was_already_told() {
        let t = Instant::now();
        let mut w = Watch::default();
        w.minimized_by_us(t);
        assert!(!w.restored(t + Duration::from_secs(3), Some(false), Some(true)));
        assert!(!w.waiting());
    }
}
