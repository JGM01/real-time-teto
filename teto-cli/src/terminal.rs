//! Raw mode is active only while waiting for activation.

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

// The wait checks the shutdown flag at this interval.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

struct RawTerminal {
    active: bool,
}

impl RawTerminal {
    fn restore(&mut self) -> io::Result<()> {
        if self.active {
            disable_raw_mode()?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("Cannot restore terminal mode: {error}");
        }
    }
}

pub(super) fn wait_for_space(shutdown: &AtomicBool) -> io::Result<bool> {
    enable_raw_mode()?;
    let mut terminal = RawTerminal { active: true };
    let result = read_activation(shutdown);
    let restored = terminal.restore();
    match result {
        Err(error) => Err(error),
        Ok(activated) => restored.map(|()| activated),
    }
}

fn read_activation(shutdown: &AtomicBool) -> io::Result<bool> {
    while !shutdown.load(Ordering::Acquire) {
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            shutdown.store(true, Ordering::Release);
            return Ok(false);
        }
        if key.code == KeyCode::Char(' ') {
            return Ok(true);
        }
    }
    Ok(false)
}
