//! Left/right jogging while the half-nut is disengaged.
//!
//! - The selected jog distance also picks a speed level (index into `speeds`):
//!   the smallest distance is the slowest.
//! - A tap moves exactly the selected jog distance at that level's speed.
//!   Further taps in the same direction add to the move.
//! - Holding the button past `hold_after_us` switches to continuous motion,
//!   starting at the same level and stepping up one level every
//!   `level_every_us` until the fastest.
//! - Releasing a hold brakes to a stop. Pressing the other direction while
//!   moving also brakes.
//!
//! Stops limit jogging the same way they limit the gearbox.

use crate::gearbox::Target;

pub const JOG_LEVELS: usize = 4;

/// Distance a hold aims ahead, in steps; it re-aims every tick so this just
/// has to exceed any braking distance.
const HOLD_LOOKAHEAD: i64 = 1 << 40;

#[derive(Clone, Copy, Debug)]
pub struct JogConfig {
    pub hold_after_us: u64,
    pub level_every_us: u64,
    /// Speed limit per level in steps/s, slowest first.
    pub speeds: [u32; JOG_LEVELS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Tap { target: i64, dir: i64, pressed_at: u64, held: bool, level: usize },
    Hold { dir: i64, since: u64, base: usize },
    Stop { target: i64, dir: i64, cap: u32 },
}

/// What the jog is doing, for the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JogView {
    #[default]
    Idle,
    /// Tap move under way; steps still to travel (after stops).
    Tap { remaining: i64 },
    /// Continuous hold; speed level 1..=JOG_LEVELS.
    Hold { level: u8 },
    /// Braking after a release or an opposite press.
    Braking,
}

/// Lower/upper position limits (right stop, left stop).
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub lo: i64,
    pub hi: i64,
}

impl Bounds {
    pub fn new(right: Option<i64>, left: Option<i64>) -> Self {
        Self { lo: right.unwrap_or(i64::MIN), hi: left.unwrap_or(i64::MAX) }
    }

    fn clamp(&self, pos: i64) -> i64 {
        pos.min(self.hi).max(self.lo)
    }
}

pub struct Jog {
    cfg: JogConfig,
    state: State,
}

impl Jog {
    pub fn new(cfg: JogConfig) -> Self {
        Self { cfg, state: State::Idle }
    }

    pub fn active(&self) -> bool {
        self.state != State::Idle
    }

    pub fn view(&self, pos: i64, bounds: Bounds, now: u64) -> JogView {
        match self.state {
            State::Idle => JogView::Idle,
            State::Tap { target, .. } => JogView::Tap { remaining: (bounds.clamp(target) - pos).abs() },
            State::Hold { .. } => JogView::Hold { level: self.level_now(now) as u8 + 1 },
            State::Stop { .. } => JogView::Braking,
        }
    }

    /// Button pressed. `dir` is +1 (left) or -1 (right); `distance` in steps;
    /// `level` is the speed level that goes with the selected distance.
    pub fn press(&mut self, dir: i64, distance: i64, level: usize, pos: i64, braking: i64, now: u64) {
        let level = level.min(JOG_LEVELS - 1);
        let tap_from =
            |start: i64| State::Tap { target: start + dir * distance, dir, pressed_at: now, held: true, level };
        self.state = match self.state {
            State::Idle => tap_from(pos),
            State::Tap { target, dir: d, .. } if d == dir => tap_from(target),
            State::Stop { target, dir: d, .. } if d == dir => tap_from(target),
            // Opposite direction or already holding: brake.
            State::Tap { dir: d, .. } | State::Hold { dir: d, .. } | State::Stop { dir: d, .. } => {
                State::Stop { target: pos + d * braking, dir: d, cap: self.cap_now(now) }
            }
        };
    }

    /// Button released.
    pub fn release(&mut self, pos: i64, braking: i64, now: u64) {
        match &mut self.state {
            State::Tap { held, .. } => *held = false,
            State::Hold { dir, .. } => {
                let dir = *dir;
                self.state = State::Stop { target: pos + dir * braking, dir, cap: self.cap_now(now) };
            }
            _ => {}
        }
    }

    /// Stop immediately (disengage / emergency).
    pub fn cancel(&mut self) {
        self.state = State::Idle;
    }

    /// Target and speed cap for this tick.
    pub fn update(&mut self, pos: i64, bounds: Bounds, now: u64) -> (Target, Option<u32>) {
        if let State::Tap { dir, pressed_at, held: true, level, .. } = self.state {
            if now - pressed_at >= self.cfg.hold_after_us {
                self.state = State::Hold { dir, since: now, base: level };
            }
        }
        match self.state {
            State::Idle => (Target::whole(pos, true), None),
            State::Tap { target, held, .. } => {
                let t = bounds.clamp(target);
                if !held && pos == t {
                    self.state = State::Idle;
                }
                (Target::whole(t, true), Some(self.cap_now(now)))
            }
            State::Hold { dir, .. } => {
                let t = bounds.clamp(pos.saturating_add(dir * HOLD_LOOKAHEAD));
                (Target::whole(t, true), Some(self.cap_now(now)))
            }
            State::Stop { target, cap, .. } => {
                let t = bounds.clamp(target);
                if pos == t {
                    self.state = State::Idle;
                }
                (Target::whole(t, true), Some(cap))
            }
        }
    }

    fn level_now(&self, now: u64) -> usize {
        match self.state {
            State::Tap { level, .. } => level,
            State::Hold { since, base, .. } => {
                let steps = ((now - since) / self.cfg.level_every_us.max(1)) as usize;
                (base + steps).min(JOG_LEVELS - 1)
            }
            _ => JOG_LEVELS - 1,
        }
    }

    fn cap_now(&self, now: u64) -> u32 {
        self.cfg.speeds[self.level_now(now)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: JogConfig =
        JogConfig { hold_after_us: 400_000, level_every_us: 1_000_000, speeds: [100, 400, 1600, 5000] };
    const FREE: Bounds = Bounds { lo: i64::MIN, hi: i64::MAX };

    #[test]
    fn tap_moves_exact_distance() {
        let mut j = Jog::new(CFG);
        j.press(1, 20, 3, 100, 0, 0);
        j.release(100, 0, 50_000);
        assert_eq!(j.update(100, FREE, 60_000), (Target::whole(120, true), Some(5000)));
        // Still held long after the move is done: no hold because it was released.
        assert_eq!(j.update(120, FREE, 900_000).0.pos, 120);
        assert!(!j.active());
    }

    #[test]
    fn repeated_taps_accumulate() {
        let mut j = Jog::new(CFG);
        j.press(-1, 20, 3, 0, 0, 0);
        j.release(0, 0, 10);
        j.press(-1, 20, 3, -5, 0, 20);
        j.release(-5, 0, 30);
        assert_eq!(j.update(-5, FREE, 40).0.pos, -40);
    }

    #[test]
    fn hold_ramps_speed_then_brakes_on_release() {
        let mut j = Jog::new(CFG);
        j.press(1, 20, 0, 0, 0, 0);
        assert_eq!(j.update(0, FREE, 399_999).1, Some(100)); // still a tap, at level 0
        let (t, cap) = j.update(10, FREE, 400_000);
        assert!(t.pos > 1_000_000);
        assert_eq!(cap, Some(100));
        assert_eq!(j.update(50, FREE, 1_400_000).1, Some(400));
        assert_eq!(j.update(500, FREE, 2_400_000).1, Some(1600));
        assert_eq!(j.update(900, FREE, 9_000_000).1, Some(5000));
        j.release(1000, 30, 9_000_001);
        assert_eq!(j.update(1000, FREE, 9_000_002), (Target::whole(1030, true), Some(5000)));
        j.update(1030, FREE, 9_000_100);
        assert!(!j.active());
    }

    #[test]
    fn view_follows_state() {
        let mut j = Jog::new(CFG);
        let b = Bounds::new(None, Some(15));
        assert_eq!(j.view(0, b, 0), JogView::Idle);
        j.press(1, 20, 0, 0, 0, 0);
        assert_eq!(j.view(5, b, 1), JogView::Tap { remaining: 10 }); // clamped by the stop at 15
        j.update(5, FREE, 400_000);
        assert_eq!(j.view(10, FREE, 400_000), JogView::Hold { level: 1 });
        assert_eq!(j.view(10, FREE, 2_500_000), JogView::Hold { level: 3 });
        assert_eq!(j.view(10, FREE, 99_000_000), JogView::Hold { level: 4 });
        j.release(10, 30, 99_000_001);
        assert_eq!(j.view(12, FREE, 99_000_002), JogView::Braking);
    }

    #[test]
    fn hold_starts_at_selected_level_and_caps_at_fastest() {
        let mut j = Jog::new(CFG);
        j.press(1, 20, 2, 0, 0, 0);
        assert_eq!(j.update(0, FREE, 1).1, Some(1600)); // tap at level 2
        assert_eq!(j.update(0, FREE, 400_000).1, Some(1600)); // hold starts there
        assert_eq!(j.update(0, FREE, 1_399_999).1, Some(1600));
        assert_eq!(j.update(0, FREE, 1_400_000).1, Some(5000)); // next level after 1 s
        assert_eq!(j.update(0, FREE, 60_000_000).1, Some(5000)); // and no further
        assert_eq!(j.view(0, FREE, 60_000_000), JogView::Hold { level: 4 });
    }

    #[test]
    fn stops_limit_jog() {
        let mut j = Jog::new(CFG);
        let b = Bounds::new(Some(-10), Some(50));
        j.press(1, 100, 3, 0, 0, 0);
        assert_eq!(j.update(0, b, 1).0.pos, 50);
        j.update(0, b, 500_000);
        assert_eq!(j.update(20, b, 500_001).0.pos, 50);
    }

    #[test]
    fn opposite_press_brakes() {
        let mut j = Jog::new(CFG);
        j.press(1, 20, 3, 0, 0, 0);
        j.update(0, FREE, 500_000); // now holding
        j.press(-1, 20, 3, 300, 12, 600_000);
        assert_eq!(j.update(300, FREE, 600_001).0.pos, 312);
    }
}
