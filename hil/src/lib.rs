//! Drives a board running the `hil` firmware build as if it were the Nextion
//! display: touch events go in over USB-Serial-JTAG, and the display commands
//! the firmware sends back are applied to a model of the screen that tests
//! assert on.
//!
//! Set `LEELS_PORT` to the board's USB serial port (e.g.
//! `/dev/cu.usbmodem1101`); without it every test is skipped. One board, so
//! tests take turns through a lock and each starts with a reset.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Component ids on page 0 of `nextion/lee-ls.HMI`, by object name. Taken
/// from the HMI (see docs/nextion.md), not from the firmware, so a mapping
/// mistake in the firmware shows up as a failing test.
const COMPONENTS: &[(&str, u8)] = &[
    ("tAngle", 2),
    ("tTurns", 3),
    ("bLeftStop", 4),
    ("bRightStop", 5),
    ("bNum1", 6),
    ("bNum2", 7),
    ("bNum3", 8),
    ("bBackspace", 9),
    ("bNum4", 10),
    ("bNum5", 11),
    ("bNum6", 12),
    ("bNum7", 13),
    ("bNum8", 14),
    ("bNum9", 15),
    ("bNum0", 16),
    ("bNumOK", 17),
    ("bNumPeriod", 18),
    ("bToggleEngaged", 19),
    ("bReverseToggle", 20),
    ("bUnitsToggle", 21),
    ("bPitch001", 23),
    ("bPitch01", 24),
    ("bPitch1", 25),
    ("bZeroZ", 26),
    ("bJogL", 27),
    ("bJogR", 28),
    ("bCycleJogDist", 29),
    ("bSettings", 31),
    ("bPitch005", 33),
    ("bPitch05", 34),
    ("bPitch5", 35),
];

/// Every field the firmware renders; the screen is complete once all are seen.
const FIELDS: &[&str] = &[
    "bToggleEngaged",
    "tPitch",
    "bUnitsToggle",
    "tRPM",
    "tTurns",
    "tAngle",
    "tZPos",
    "bLeftStop",
    "bRightStop",
    "bCycleJogDist",
    "tMessageLine",
    "bReverseToggle",
];

/// RGB565 background colours the firmware uses.
pub const GREY: &str = "50712";
pub const AMBER: &str = "64896";
pub const DARK_RED: &str = "40960";
pub const PITCH_NORMAL_BG: &str = "10597";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

static BOARD: Mutex<()> = Mutex::new(());

pub struct Board {
    port: Box<dyn serialport::SerialPort>,
    partial: Vec<u8>,
    /// `name` → text, and `name.bco` / `name.pco` → colour, as last sent.
    screen: HashMap<String, String>,
    /// (planned position, pulses counted on the STEP pin), in steps.
    steps: (i64, i64),
    steps_changed: Instant,
    /// Recent log lines, shown when a wait times out.
    history: Vec<String>,
    _lock: MutexGuard<'static, ()>,
}

impl Board {
    /// Reset the board and wait for the first full screen. `None` (and a
    /// note on stderr) when `LEELS_PORT` isn't set.
    pub fn open() -> Option<Board> {
        let Ok(path) = std::env::var("LEELS_PORT") else {
            eprintln!("LEELS_PORT not set: skipping hardware test");
            return None;
        };
        let lock = BOARD.lock().unwrap_or_else(|e| e.into_inner());
        let port = serialport::new(&path, 115_200)
            .timeout(Duration::from_millis(20))
            .open()
            .unwrap_or_else(|e| panic!("open {path}: {e}"));
        let mut board = Board {
            port,
            partial: Vec::new(),
            screen: HashMap::new(),
            steps: (0, 0),
            steps_changed: Instant::now(),
            history: Vec::new(),
            _lock: lock,
        };
        board.reset();
        Some(board)
    }

    /// Hardware reset through the USB-Serial-JTAG's RTS line (DTR released so
    /// it boots the firmware, not the ROM loader).
    fn reset(&mut self) {
        self.port.write_data_terminal_ready(false).unwrap();
        self.port.write_request_to_send(true).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        self.port.write_request_to_send(false).unwrap();
        let _ = self.port.clear(serialport::ClearBuffer::Input);
        self.partial.clear();
        self.history.clear();
        self.screen.clear();
        self.steps = (0, 0);
        self.steps_changed = Instant::now();
        self.wait_until("the firmware to boot", Duration::from_secs(8), |b| {
            b.history.iter().any(|l| l == "hil: ready")
        });
        self.wait_until("the first full screen", DEFAULT_TIMEOUT, |b| FIELDS.iter().all(|f| b.screen.contains_key(*f)));
    }

    fn send(&mut self, name: &str, pressed: bool) {
        let id =
            COMPONENTS.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no touchable component {name}")).1;
        // Nextion touch event: 0x65 page component event, then FF FF FF.
        self.port.write_all(&[0x65, 0, id, pressed as u8, 0xFF, 0xFF, 0xFF]).unwrap();
        self.port.flush().unwrap();
    }

    pub fn press(&mut self, name: &str) {
        self.send(name, true);
        self.pump(Duration::from_millis(30));
    }

    pub fn release(&mut self, name: &str) {
        self.send(name, false);
        self.pump(Duration::from_millis(30));
    }

    /// Press and release, as a short tap on the screen.
    pub fn tap(&mut self, name: &str) {
        self.press(name);
        self.pump(Duration::from_millis(50));
        self.release(name);
    }

    /// Tap each component in turn.
    pub fn tap_all(&mut self, names: &[&str]) {
        for n in names {
            self.tap(n);
        }
    }

    /// Hold a component for `held`, then release it.
    pub fn hold(&mut self, name: &str, held: Duration) {
        self.press(name);
        self.pump(held);
        self.release(name);
    }

    /// What the firmware last put in a field (`"tZPos"`, or `"tPitch.bco"`).
    pub fn field(&self, name: &str) -> &str {
        self.screen.get(name).map(String::as_str).unwrap_or("")
    }

    /// Wait until `field` equals `expected`; panics with the recent log if it doesn't.
    pub fn expect(&mut self, field: &str, expected: &str) {
        self.wait_until(&format!("{field} == {expected:?}"), DEFAULT_TIMEOUT, |b| b.field(field) == expected);
    }

    /// Wait until `field` starts with `prefix`.
    pub fn expect_prefix(&mut self, field: &str, prefix: &str) {
        self.wait_until(&format!("{field} starts with {prefix:?}"), DEFAULT_TIMEOUT, |b| {
            b.field(field).starts_with(prefix)
        });
    }

    /// Wait for motion to finish and every planned step to reach the STEP
    /// pin. Returns the position in steps.
    pub fn settled_steps(&mut self) -> i64 {
        self.wait_until("steps to settle with planned == pulses", Duration::from_secs(10), |b| {
            b.steps.0 == b.steps.1 && b.steps_changed.elapsed() >= Duration::from_millis(300)
        });
        self.steps.0
    }

    /// Read and apply the log for `d`.
    pub fn pump(&mut self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.read_some();
        }
    }

    fn wait_until(&mut self, what: &str, timeout: Duration, mut done: impl FnMut(&Board) -> bool) {
        let end = Instant::now() + timeout;
        while !done(self) {
            if Instant::now() >= end {
                let tail = self.history.len().saturating_sub(40);
                panic!(
                    "timed out after {timeout:?} waiting for {what}\nscreen: {:#?}\nsteps (planned, pulses): {:?}\nlast log lines:\n{}",
                    self.screen,
                    self.steps,
                    self.history[tail..].join("\n")
                );
            }
            self.read_some();
        }
    }

    fn read_some(&mut self) {
        let mut buf = [0u8; 1024];
        let n = match self.port.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return,
            Err(e) => panic!("serial read: {e}"),
        };
        self.partial.extend_from_slice(&buf[..n]);
        while let Some(i) = self.partial.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.partial.drain(..=i).collect();
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            self.apply(&line);
            self.history.push(line);
        }
    }

    fn apply(&mut self, line: &str) {
        if let Some(cmd) = line.strip_prefix("nextion tx: ") {
            if let Some((name, rest)) = cmd.split_once(".txt=\"") {
                let text = rest.strip_suffix('"').unwrap_or(rest).replace("\\xdf", "°");
                self.screen.insert(name.to_string(), text);
            } else if let Some((name_attr, value)) = cmd.split_once('=') {
                self.screen.insert(name_attr.to_string(), value.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("hil: steps ") {
            let num = |key: &str| {
                rest.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(key))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| panic!("bad steps line: {line}"))
            };
            self.steps = (num("planned="), num("pulses="));
            self.steps_changed = Instant::now();
        }
    }
}
