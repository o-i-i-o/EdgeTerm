//! The main window's size across launches. Every `Resized` event is folded
//! into a snapshot of the last size the window had as an ordinary window
//! (`remember`); the snapshot goes to `appearance.json` once the window has
//! held still for a moment and again when it closes, and the next launch
//! creates the window at it, fitted to the display it will open on (`fit`).

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use parking_lot::Mutex;

use crate::store::{self, WindowGeometry};

/// How long a resize has to have stopped before the size is written: a drag
/// on the window edge reports dozens of sizes a second.
const SETTLE: Duration = Duration::from_millis(500);

/// What a `Resized` event says about the window, in logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub width: u32,
    pub height: u32,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// Folds an observation into the remembered geometry. Only an ordinary
/// window's size is worth keeping: a minimized window reports 0×0 on
/// Windows and a maximized or fullscreen one reports the screen, and either
/// would come back as an empty or a screen-sized ordinary window. Maximized
/// is kept as a flag on top of the last ordinary size, which is what the
/// window restores to; fullscreen is not restored at all, the way macOS
/// applications behave.
pub fn remember(previous: Option<WindowGeometry>, seen: Observation) -> Option<WindowGeometry> {
    if seen.minimized || seen.fullscreen || seen.width == 0 || seen.height == 0 {
        return previous;
    }
    if seen.maximized {
        return previous.map(|geometry| WindowGeometry {
            maximized: true,
            ..geometry
        });
    }
    Some(WindowGeometry {
        width: seen.width,
        height: seen.height,
        maximized: false,
    })
}

/// Fits a remembered size to this launch. A size remembered on a larger
/// display that is since unplugged would spill off the screen (AppKit only
/// pulls the height in, Windows nothing), so it is capped at the work area
/// of the display the window opens on; and it never goes below the window's
/// minimum size, which the OS would enforce on the first resize anyway.
pub fn fit(
    remembered: WindowGeometry,
    min: (u32, u32),
    work_area: Option<(u32, u32)>,
) -> WindowGeometry {
    let (mut width, mut height) = (remembered.width, remembered.height);
    if let Some((max_width, max_height)) = work_area {
        width = width.min(max_width);
        height = height.min(max_height);
    }
    WindowGeometry {
        width: width.max(min.0),
        height: height.max(min.1),
        maximized: remembered.maximized,
    }
}

/// The snapshot and the thread that writes it.
pub struct WindowMemory {
    latest: Mutex<Option<WindowGeometry>>,
    writer: Sender<WindowGeometry>,
}

impl WindowMemory {
    /// `initial` is what the window is created at, so a launch that comes up
    /// maximized straight away still knows the size underneath.
    pub fn new(initial: Option<WindowGeometry>) -> Self {
        let (writer, queue) = mpsc::channel::<WindowGeometry>();
        std::thread::spawn(move || {
            loop {
                let Ok(mut pending) = queue.recv() else {
                    return;
                };
                // Settled, or the application is going away: write either way.
                while let Ok(next) = queue.recv_timeout(SETTLE) {
                    pending = next;
                }
                let _ = store::save_window_geometry(pending);
            }
        });
        Self {
            latest: Mutex::new(initial),
            writer,
        }
    }

    /// Folds a `Resized` event in and queues the write.
    pub fn record(&self, seen: Observation) {
        let mut latest = self.latest.lock();
        let next = remember(*latest, seen);
        if next == *latest {
            return;
        }
        *latest = next;
        if let Some(geometry) = next {
            let _ = self.writer.send(geometry);
        }
    }

    /// Writes the snapshot now, for the moments the writer thread may not get
    /// to: the window closing and the application exiting.
    pub fn flush(&self) {
        if let Some(geometry) = *self.latest.lock() {
            let _ = store::save_window_geometry(geometry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordinary(width: u32, height: u32) -> Observation {
        Observation {
            width,
            height,
            minimized: false,
            maximized: false,
            fullscreen: false,
        }
    }

    const REMEMBERED: WindowGeometry = WindowGeometry {
        width: 1200,
        height: 800,
        maximized: false,
    };

    #[test]
    fn an_ordinary_resize_replaces_the_remembered_size() {
        assert_eq!(
            remember(None, ordinary(1000, 700)),
            Some(WindowGeometry {
                width: 1000,
                height: 700,
                maximized: false
            })
        );
        assert_eq!(
            remember(Some(REMEMBERED), ordinary(1000, 700)),
            Some(WindowGeometry {
                width: 1000,
                height: 700,
                maximized: false
            })
        );
        // Restoring a maximized window clears the flag along with the size.
        let maximized = WindowGeometry {
            maximized: true,
            ..REMEMBERED
        };
        assert_eq!(
            remember(Some(maximized), ordinary(1200, 800)),
            Some(REMEMBERED)
        );
    }

    #[test]
    fn maximized_keeps_the_ordinary_size_underneath() {
        let seen = Observation {
            maximized: true,
            ..ordinary(2560, 1415)
        };
        assert_eq!(
            remember(Some(REMEMBERED), seen),
            Some(WindowGeometry {
                maximized: true,
                ..REMEMBERED
            })
        );
        // With nothing underneath there is nothing to restore to.
        assert_eq!(remember(None, seen), None);
    }

    #[test]
    fn minimized_fullscreen_and_empty_sizes_change_nothing() {
        let minimized = Observation {
            minimized: true,
            ..ordinary(0, 0)
        };
        let fullscreen = Observation {
            fullscreen: true,
            ..ordinary(2560, 1440)
        };
        let empty = ordinary(0, 600);
        for seen in [minimized, fullscreen, empty] {
            assert_eq!(remember(Some(REMEMBERED), seen), Some(REMEMBERED));
            assert_eq!(remember(None, seen), None);
        }
        // Fullscreen on macOS also reads as zoomed; the flag must not stick.
        let zoomed_fullscreen = Observation {
            maximized: true,
            ..fullscreen
        };
        assert_eq!(
            remember(Some(REMEMBERED), zoomed_fullscreen),
            Some(REMEMBERED)
        );
    }

    #[test]
    fn a_remembered_size_is_fitted_to_the_display_and_the_minimum() {
        let min = (900, 560);
        // A size from a larger display shrinks to the work area.
        let large = WindowGeometry {
            width: 2400,
            height: 1300,
            maximized: true,
        };
        assert_eq!(
            fit(large, min, Some((1440, 875))),
            WindowGeometry {
                width: 1440,
                height: 875,
                maximized: true
            }
        );
        // One that fits is left alone, with or without a known work area.
        assert_eq!(fit(REMEMBERED, min, Some((1440, 875))), REMEMBERED);
        assert_eq!(fit(REMEMBERED, min, None), REMEMBERED);
        // Below the minimum (a hand-edited file, a smaller earlier build) the
        // minimum wins, even over a tiny work area.
        let tiny = WindowGeometry {
            width: 100,
            height: 100,
            maximized: false,
        };
        assert_eq!(
            fit(tiny, min, Some((800, 500))),
            WindowGeometry {
                width: 900,
                height: 560,
                maximized: false
            }
        );
    }
}
