//! The text log of the web interface: the last lines logged with [`wlog!`], numbered, each also
//! logged through defmt.

use core::{cell::RefCell, fmt::Write};

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::Instant;
use heapless::{Deque, String};

/// Lines kept, and their length.
const LINES: usize = 48;
pub type Line = String<128>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
    Info,
}

struct Log {
    lines: Deque<(u32, Line), LINES>,
    next: u32,
}

static LOG: Mutex<CriticalSectionRawMutex, RefCell<Log>> = Mutex::new(RefCell::new(Log {
    lines: Deque::new(),
    next: 1,
}));

/// Adds a line `L (uptime ms) tag: message`, `L` being the level letter.
pub fn push(level: Level, tag: &str, args: core::fmt::Arguments) {
    let letter = match level {
        Level::Error => 'E',
        Level::Warn => 'W',
        Level::Info => 'I',
    };
    let mut line = Line::new();
    let _ = write!(line, "{letter} ({}) {tag}: ", Instant::now().as_millis());
    let mut rest = Truncating(&mut line);
    let _ = rest.write_fmt(args);
    match level {
        Level::Error => defmt::error!("{}", line.as_str()),
        Level::Warn => defmt::warn!("{}", line.as_str()),
        Level::Info => defmt::info!("{}", line.as_str()),
    }
    LOG.lock(|log| {
        let mut log = log.borrow_mut();
        let n = log.next;
        log.next = n.wrapping_add(1);
        if log.lines.is_full() {
            log.lines.pop_front();
        }
        let _ = log.lines.push_back((n, line));
    });
}

/// Calls `f` with each line numbered after `after`, oldest first. Returns the number of the
/// last line.
pub fn for_each_after(after: u32, mut f: impl FnMut(u32, &str)) -> u32 {
    LOG.lock(|log| {
        let log = log.borrow();
        for (n, line) in log.lines.iter() {
            if *n > after {
                f(*n, line);
            }
        }
        log.next.wrapping_sub(1)
    })
}

/// Writes what fits, dropping the rest.
struct Truncating<'a, const N: usize>(&'a mut String<N>);

impl<const N: usize> Write for Truncating<'_, N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            if self.0.push(c).is_err() {
                break;
            }
        }
        Ok(())
    }
}

/// Logs a line for the web interface and through defmt: `wlog!(Info, "pool", "connected to
/// {}", host)`.
#[macro_export]
macro_rules! wlog {
    ($level:ident, $tag:literal, $($arg:tt)*) => {
        $crate::weblog::push($crate::weblog::Level::$level, $tag, format_args!($($arg)*))
    };
}
