//! Operator interface logic: turns key presses into pitch state and commands
//! for the motion task. Knows nothing about the display hardware.

use crate::display::Status;
use crate::gearbox::Side;
use crate::pitch::{parse_entry_milli, Pitch, Unit, JOG_SIZES_MILLI, STEP_SIZES_MILLI};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Point,
    Backspace,
    Enter,
    Plus,
    Minus,
    Reverse,
    ToggleUnit,
    /// Select pitch increment by index into [`STEP_SIZES_MILLI`].
    StepSize(u8),
    CycleStep,
    Engage,
    Disengage,
    StopLeft,
    StopRight,
    ZeroZ,
    ZeroTurns,
    /// Jog button pressed or released (`left` = towards the left stop).
    Jog {
        left: bool,
        pressed: bool,
    },
    /// Cycle jog distance through [`JOG_SIZES_MILLI`].
    JogCycle,
}

/// Commands for the motion task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SetPitchDu(i64),
    Engage,
    Disengage,
    ToggleStop(Side),
    ZeroZ,
    ZeroTurns,
    Jog { left: bool, distance_du: i64 },
    JogRelease,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub command: Option<Command>,
    pub beep: bool,
}

impl Outcome {
    fn cmd(c: Command) -> Self {
        Self { command: Some(c), beep: false }
    }
}

const MESSAGE_MS: u64 = 2_000;
const ENTRY_MAX_LEN: usize = 8;

pub struct Ui {
    pub pitch: Pitch,
    step_idx: usize,
    jog_idx: usize,
    entry: Option<heapless::String<ENTRY_MAX_LEN>>,
    message: Option<(&'static str, u64)>,
    max_pitch_du: i64,
}

impl Ui {
    pub fn new(max_pitch_du: i64) -> Self {
        Self {
            pitch: Pitch { magnitude_milli: 0, reversed: false, unit: Unit::Mm },
            step_idx: 2,
            jog_idx: 1,
            entry: None,
            message: None,
            max_pitch_du,
        }
    }

    pub fn step_milli(&self) -> u32 {
        STEP_SIZES_MILLI[self.step_idx]
    }

    pub fn jog_milli(&self) -> u32 {
        JOG_SIZES_MILLI[self.jog_idx]
    }

    /// Number being typed on the keypad, if any.
    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    /// Transient feedback message, if one is still showing.
    pub fn message(&self, now_ms: u64) -> Option<&'static str> {
        self.message.filter(|&(_, until)| now_ms < until).map(|(m, _)| m)
    }

    /// `status` is the latest motion snapshot, used to refuse keys that
    /// don't make sense right now.
    pub fn handle(&mut self, key: Key, now_ms: u64, status: &Status) -> Outcome {
        match key {
            Key::Digit(d) => self.edit(now_ms, |e| {
                let frac = e.split_once('.').map_or(0, |(_, f)| f.len());
                if frac >= 3 {
                    return false;
                }
                e.push((b'0' + d) as char).is_ok()
            }),
            Key::Point => self.edit(now_ms, |e| !e.contains('.') && e.push('.').is_ok()),
            Key::Backspace => {
                if let Some(e) = &mut self.entry {
                    e.pop();
                    if e.is_empty() {
                        self.entry = None;
                    }
                }
                Outcome::default()
            }
            Key::Enter => match self.entry.take() {
                None => Outcome::default(),
                Some(e) => match parse_entry_milli(&e) {
                    Some(m) => self.try_set(Pitch { magnitude_milli: m, ..self.pitch }, now_ms),
                    None => self.reject("Invalid number", now_ms),
                },
            },
            // Don't engage with a half-typed pitch on screen.
            Key::Engage if self.entry.is_some() => self.reject("Press Enter first", now_ms),
            Key::Engage if status.jogging => self.reject("Wait for jog to stop", now_ms),
            Key::Engage => Outcome::cmd(Command::Engage),
            // Always forward releases so a jog can never be left running.
            Key::Jog { pressed: false, .. } => Outcome::cmd(Command::JogRelease),
            Key::Disengage => {
                self.entry = None;
                Outcome::cmd(Command::Disengage)
            }
            other => {
                // Any other key abandons a pending entry.
                self.entry = None;
                self.handle_other(other, now_ms, status)
            }
        }
    }

    fn handle_other(&mut self, key: Key, now_ms: u64, status: &Status) -> Outcome {
        let p = self.pitch;
        match key {
            Key::Plus => {
                let m = p.magnitude_milli.saturating_add(self.step_milli());
                self.try_set(Pitch { magnitude_milli: m, ..p }, now_ms)
            }
            Key::Minus => {
                let m = p.magnitude_milli.saturating_sub(self.step_milli());
                self.try_set(Pitch { magnitude_milli: m, ..p }, now_ms)
            }
            Key::Reverse => self.try_set(Pitch { reversed: !p.reversed, ..p }, now_ms),
            Key::ToggleUnit => self.try_set(Pitch { unit: p.unit.toggled(), ..p }, now_ms),
            Key::StepSize(i) => {
                self.step_idx = (i as usize).min(STEP_SIZES_MILLI.len() - 1);
                Outcome::default()
            }
            Key::CycleStep => {
                self.step_idx = (self.step_idx + 1) % STEP_SIZES_MILLI.len();
                Outcome::default()
            }
            Key::StopLeft => Outcome::cmd(Command::ToggleStop(Side::Left)),
            Key::StopRight => Outcome::cmd(Command::ToggleStop(Side::Right)),
            Key::ZeroZ => Outcome::cmd(Command::ZeroZ),
            Key::ZeroTurns => Outcome::cmd(Command::ZeroTurns),
            Key::JogCycle => {
                self.jog_idx = (self.jog_idx + 1) % JOG_SIZES_MILLI.len();
                Outcome::default()
            }
            Key::Jog { .. } if status.engaged => self.reject("Disengage to jog", now_ms),
            Key::Jog { left, .. } => {
                let distance_du = self.jog_milli() as i64 * p.unit.du_per_milli();
                Outcome::cmd(Command::Jog { left, distance_du })
            }
            Key::Digit(_) | Key::Point | Key::Backspace | Key::Enter | Key::Engage | Key::Disengage => {
                unreachable!("handled in Ui::handle")
            }
        }
    }

    fn edit(&mut self, now_ms: u64, f: impl FnOnce(&mut heapless::String<ENTRY_MAX_LEN>) -> bool) -> Outcome {
        let entry = self.entry.get_or_insert_with(heapless::String::new);
        if f(entry) {
            Outcome::default()
        } else {
            if entry.is_empty() {
                self.entry = None;
            }
            self.reject("Can't add that", now_ms)
        }
    }

    fn try_set(&mut self, p: Pitch, now_ms: u64) -> Outcome {
        if p.du().abs() > self.max_pitch_du {
            return self.reject("Pitch too large", now_ms);
        }
        if p == self.pitch {
            return Outcome::default();
        }
        self.pitch = p;
        Outcome::cmd(Command::SetPitchDu(p.du()))
    }

    fn reject(&mut self, msg: &'static str, now_ms: u64) -> Outcome {
        self.message = Some((msg, now_ms + MESSAGE_MS));
        Outcome { command: None, beep: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: i64 = 254_000;

    const IDLE: Status = Status {
        engaged: false,
        syncing: false,
        jogging: false,
        pos: 0,
        z_zero: 0,
        left_stop: None,
        right_stop: None,
        rpm: 0,
        turn_counts: 0,
    };

    fn press(ui: &mut Ui, keys: &[Key]) -> Vec<Outcome> {
        keys.iter().map(|&k| ui.handle(k, 0, &IDLE)).collect()
    }

    #[test]
    fn plus_minus_use_selected_step() {
        let mut ui = Ui::new(MAX);
        let out = press(&mut ui, &[Key::StepSize(3), Key::Plus, Key::StepSize(0), Key::Plus, Key::Plus]);
        assert_eq!(out[1].command, Some(Command::SetPitchDu(10_000)));
        assert_eq!(ui.pitch.magnitude_milli, 1_002);
        press(&mut ui, &[Key::StepSize(3), Key::Minus, Key::Minus]);
        assert_eq!(ui.pitch.magnitude_milli, 0);
    }

    #[test]
    fn reverse_flips_sign() {
        let mut ui = Ui::new(MAX);
        press(&mut ui, &[Key::StepSize(3), Key::Plus]);
        let out = ui.handle(Key::Reverse, 0, &IDLE);
        assert_eq!(out.command, Some(Command::SetPitchDu(-10_000)));
        // +/- change the magnitude and keep the direction.
        assert_eq!(ui.handle(Key::Plus, 0, &IDLE).command, Some(Command::SetPitchDu(-20_000)));
    }

    #[test]
    fn unit_toggle_keeps_number() {
        let mut ui = Ui::new(MAX);
        press(&mut ui, &[Key::StepSize(2), Key::Plus]);
        assert_eq!(ui.handle(Key::ToggleUnit, 0, &IDLE).command, Some(Command::SetPitchDu(25_400)));
        assert_eq!(ui.pitch.magnitude_milli, 100);
        assert_eq!(ui.pitch.unit, Unit::Inch);
    }

    #[test]
    fn unit_toggle_refused_when_too_large() {
        let mut ui = Ui::new(MAX);
        press(&mut ui, &[Key::StepSize(4), Key::Plus]); // 10mm
        let out = ui.handle(Key::ToggleUnit, 0, &IDLE);
        assert!(out.beep);
        assert_eq!(ui.pitch.unit, Unit::Mm);
        assert_eq!(ui.message(0), Some("Pitch too large"));
        assert_eq!(ui.message(MESSAGE_MS), None);
    }

    #[test]
    fn keypad_entry() {
        let mut ui = Ui::new(MAX);
        let out = press(&mut ui, &[Key::Digit(1), Key::Point, Key::Digit(2), Key::Digit(5)]);
        assert!(out.iter().all(|o| o.command.is_none()));
        assert_eq!(ui.entry(), Some("1.25"));
        assert_eq!(ui.handle(Key::Enter, 0, &IDLE).command, Some(Command::SetPitchDu(12_500)));
        assert_eq!(ui.entry(), None);
    }

    #[test]
    fn keypad_limits_decimals_and_backspace() {
        let mut ui = Ui::new(MAX);
        let out = press(&mut ui, &[Key::Point, Key::Digit(1), Key::Digit(2), Key::Digit(3), Key::Digit(4)]);
        assert!(out[4].beep);
        assert_eq!(ui.entry(), Some(".123"));
        press(&mut ui, &[Key::Backspace, Key::Backspace, Key::Backspace, Key::Backspace]);
        assert_eq!(ui.entry(), None);
        assert!(ui.handle(Key::Point, 0, &IDLE) == Outcome::default());
        assert!(ui.handle(Key::Point, 0, &IDLE).beep);
    }

    #[test]
    fn engage_blocked_during_entry_and_other_keys_cancel_it() {
        let mut ui = Ui::new(MAX);
        ui.handle(Key::Digit(2), 0, &IDLE);
        assert!(ui.handle(Key::Engage, 0, &IDLE).beep);
        assert_eq!(ui.handle(Key::StopLeft, 0, &IDLE).command, Some(Command::ToggleStop(Side::Left)));
        assert_eq!(ui.entry(), None);
        assert_eq!(ui.handle(Key::Engage, 0, &IDLE).command, Some(Command::Engage));
    }

    #[test]
    fn jog_press_release_and_cycle() {
        let mut ui = Ui::new(MAX);
        let left = |pressed| Key::Jog { left: true, pressed };
        assert_eq!(ui.handle(left(true), 0, &IDLE).command, Some(Command::Jog { left: true, distance_du: 1_000 }));
        assert_eq!(ui.handle(left(false), 0, &IDLE).command, Some(Command::JogRelease));
        press(&mut ui, &[Key::JogCycle, Key::ToggleUnit]);
        assert_eq!(ui.jog_milli(), 1_000);
        assert_eq!(ui.handle(left(true), 0, &IDLE).command, Some(Command::Jog { left: true, distance_du: 254_000 }));
        let engaged = Status { engaged: true, ..IDLE };
        assert!(ui.handle(left(true), 0, &engaged).beep);
        assert_eq!(ui.handle(left(false), 0, &engaged).command, Some(Command::JogRelease));
        let jogging = Status { jogging: true, ..IDLE };
        assert!(ui.handle(Key::Engage, 0, &jogging).beep);
    }

    #[test]
    fn entry_keeps_direction_and_rejects_too_large() {
        let mut ui = Ui::new(MAX);
        press(&mut ui, &[Key::Plus, Key::Reverse]);
        press(&mut ui, &[Key::Digit(2)]);
        assert_eq!(ui.handle(Key::Enter, 0, &IDLE).command, Some(Command::SetPitchDu(-20_000)));
        press(&mut ui, &[Key::Digit(3), Key::Digit(0)]);
        assert!(ui.handle(Key::Enter, 0, &IDLE).beep);
        assert_eq!(ui.pitch.magnitude_milli, 2_000);
    }
}
