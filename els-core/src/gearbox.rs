//! The virtual half-nut: maps spindle position to a Z motor target position.
//!
//! While engaged, `target = p_ref + (spindle - s_ref) * num / den` where
//! `num / den` is motor steps per spindle count for the current pitch. The
//! reference point is only reset when the operator engages or changes the
//! pitch, so positions never drift and the thread stays in phase through
//! stops and spindle reversals.
//!
//! Stops clamp the target. While resting on a stop, whole spindle turns of
//! overshoot are discarded (keeping thread phase) so that reversing the
//! spindle brings the carriage back within less than one turn. Removing a stop
//! the carriage is resting on puts the gearbox into "sync": the carriage holds
//! until the spindle reaches the thread phase again, then continues.

use crate::Machine;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Upper bound of the Z position (towards the headstock by default).
    Left,
    /// Lower bound of the Z position.
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// Whole steps (floor of the exact position).
    pub pos: i64,
    /// Fraction of a step beyond `pos`, in 1/2^32 steps. Lets the step
    /// generator space pulses evenly when the ratio isn't a whole number of
    /// steps per control period.
    pub frac: u32,
    /// True when the target won't advance with the spindle (disengaged, on a
    /// stop, waiting for sync). The stepper may brake towards it.
    pub is_final: bool,
}

impl Target {
    pub const fn whole(pos: i64, is_final: bool) -> Self {
        Self { pos, frac: 0, is_final }
    }
}

pub struct Gearbox {
    m: Machine,
    /// Motor steps per spindle count, as a reduced fraction. `den > 0`.
    num: i64,
    den: i64,
    engaged: bool,
    s_ref: i64,
    p_ref: i64,
    left: Option<i64>,
    right: Option<i64>,
    /// Spindle turn quotient at the moment sync started; `Some` while syncing.
    sync_q: Option<i64>,
    target: i64,
}

impl Gearbox {
    pub fn new(m: Machine) -> Self {
        Self { m, num: 0, den: 1, engaged: false, s_ref: 0, p_ref: 0, left: None, right: None, sync_q: None, target: 0 }
    }

    pub fn engaged(&self) -> bool {
        self.engaged
    }

    pub fn syncing(&self) -> bool {
        self.sync_q.is_some()
    }

    pub fn stop(&self, side: Side) -> Option<i64> {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    /// Change the pitch. If engaged, motion continues from the current target
    /// at the new ratio (thread phase relative to earlier passes is lost when
    /// the magnitude changes, as with a mechanical gearbox).
    pub fn set_pitch_du(&mut self, pitch_du: i64, spindle: i64) {
        let num = pitch_du * self.m.motor_steps;
        let den = self.m.screw_du * self.m.counts_per_rev;
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()).max(1) as i64;
        if self.engaged {
            self.p_ref = self.target;
            self.s_ref = spindle;
            self.sync_q = None;
        }
        self.num = num / g;
        self.den = den / g;
    }

    /// Close the virtual half-nut at the current motor position.
    pub fn engage(&mut self, spindle: i64, pos: i64) {
        self.engaged = true;
        self.s_ref = spindle;
        self.p_ref = pos;
        self.target = pos;
        self.sync_q = None;
    }

    /// Open the virtual half-nut; the motor stops where it is.
    pub fn disengage(&mut self, pos: i64) {
        self.engaged = false;
        self.sync_q = None;
        self.target = pos;
    }

    /// Set a stop at `pos` if none is set, otherwise remove it.
    pub fn toggle_stop(&mut self, side: Side, spindle: i64, pos: i64) {
        let slot = match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        };
        match *slot {
            Some(stop) => {
                *slot = None;
                if self.engaged && self.num != 0 && self.target == stop {
                    let d = spindle - self.spindle_at(self.target);
                    self.sync_q = Some(d.div_euclid(self.m.counts_per_rev));
                }
            }
            None => *slot = Some(pos),
        }
    }

    /// Compute the motor target for the current (backlash-filtered) spindle
    /// position. Call on every motion tick.
    pub fn update(&mut self, spindle: i64) -> Target {
        if !self.engaged || self.num == 0 {
            return Target::whole(self.target, true);
        }
        self.normalize(spindle);
        let n = self.m.counts_per_rev;

        if let Some(q0) = self.sync_q {
            let d = spindle - self.spindle_at(self.target);
            let q = d.div_euclid(n);
            let boundary = if d.rem_euclid(n) == 0 {
                Some(d)
            } else if q != q0 {
                Some(q.max(q0) * n)
            } else {
                None
            };
            match boundary {
                Some(b) => {
                    self.s_ref += b;
                    self.sync_q = None;
                }
                None => return Target::whole(self.target, true),
            }
        }

        let mut unclamped = self.unclamped(spindle);
        let clamped = self.clamp(unclamped);
        if clamped != unclamped {
            // Resting on a stop: drop whole turns of overshoot.
            let d = spindle - self.spindle_at(clamped);
            let whole = (d / n) * n;
            if whole != 0 {
                self.s_ref += whole;
                unclamped = self.unclamped(spindle);
            }
        }
        self.target = self.clamp(unclamped);
        if self.target != unclamped {
            return Target::whole(self.target, true);
        }
        Target { pos: self.target, frac: self.unclamped_frac(spindle), is_final: false }
    }

    fn clamp(&self, mut pos: i64) -> i64 {
        if let Some(l) = self.left {
            pos = pos.min(l);
        }
        if let Some(r) = self.right {
            pos = pos.max(r);
        }
        pos
    }

    fn unclamped(&self, spindle: i64) -> i64 {
        let d = spindle - self.s_ref;
        let steps = match d.checked_mul(self.num) {
            Some(n) => n.div_euclid(self.den),
            None => (d as i128 * self.num as i128).div_euclid(self.den as i128) as i64,
        };
        self.p_ref + steps
    }

    /// Fractional step beyond `unclamped(spindle)`, in 1/2^32 steps.
    fn unclamped_frac(&self, spindle: i64) -> u32 {
        let d = spindle - self.s_ref;
        let rem = match d.checked_mul(self.num) {
            Some(n) => n.rem_euclid(self.den),
            None => (d as i128 * self.num as i128).rem_euclid(self.den as i128) as i64,
        };
        // rem < den; stay in 64-bit arithmetic when the shift fits.
        if rem < 1 << 31 {
            (((rem as u64) << 32) / self.den as u64) as u32
        } else {
            (((rem as u128) << 32) / self.den as u128) as u32
        }
    }

    /// Spindle count at which the (unclamped) target equals `pos`.
    fn spindle_at(&self, pos: i64) -> i64 {
        self.s_ref + ((pos - self.p_ref) as i128 * self.den as i128 / self.num as i128) as i64
    }

    /// Keep `spindle - s_ref` small by moving the reference along the line by
    /// whole multiples of `den` (exact, no rounding).
    fn normalize(&mut self, spindle: i64) {
        let d = spindle - self.s_ref;
        if d.abs() > 1 << 30 {
            let q = d / self.den;
            self.s_ref += q * self.den;
            self.p_ref += q * self.num;
        }
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1200 PPR encoder, 800 steps/rev, 4mm screw: 200 steps/mm.
    const M: Machine = Machine { counts_per_rev: 2400, motor_steps: 800, screw_du: 40_000 };

    fn engaged_1mm() -> Gearbox {
        let mut g = Gearbox::new(M);
        g.set_pitch_du(10_000, 0);
        g.engage(0, 0);
        g
    }

    #[test]
    fn follows_spindle() {
        let mut g = engaged_1mm();
        assert_eq!(g.update(1200), Target::whole(100, false));
        // 1/12 step per count: 6 counts is half a step.
        assert_eq!(g.update(1206), Target { pos: 100, frac: 1 << 31, is_final: false });
        assert_eq!(g.update(2400).pos, 200);
        assert_eq!(g.update(-2400).pos, -200);
    }

    #[test]
    fn disengaged_holds() {
        let mut g = Gearbox::new(M);
        g.set_pitch_du(10_000, 0);
        assert_eq!(g.update(5000), Target::whole(0, true));
        g.engage(5000, 42);
        assert_eq!(g.update(5000 + 2400).pos, 242);
        g.disengage(242);
        assert_eq!(g.update(20_000).pos, 242);
    }

    #[test]
    fn reversed_pitch_moves_other_way() {
        let mut g = Gearbox::new(M);
        g.set_pitch_du(-10_000, 0);
        g.engage(0, 0);
        assert_eq!(g.update(2400).pos, -200);
    }

    #[test]
    fn stop_discards_whole_turns_and_keeps_phase() {
        let mut g = engaged_1mm();
        g.toggle_stop(Side::Left, 0, 100); // arrival at s=1200
        assert_eq!(g.update(7200), Target::whole(100, true));
        // Reverse: leaves the stop at s=6000, i.e. 1200 mod 2400 like the arrival.
        assert_eq!(g.update(6100).pos, 100);
        assert_eq!(g.update(5760).pos, 80);
    }

    #[test]
    fn removing_stop_waits_for_thread_phase() {
        let mut g = engaged_1mm();
        g.toggle_stop(Side::Left, 0, 100);
        g.update(7200);
        g.toggle_stop(Side::Left, 7200, 100);
        assert!(g.syncing());
        assert_eq!(g.update(7300), Target::whole(100, true));
        assert_eq!(g.update(8399).pos, 100);
        // Phase reached (8400 = 1200 mod 2400): continue from the stop position.
        assert_eq!(g.update(8400).pos, 100);
        assert!(!g.syncing());
        assert_eq!(g.update(8520).pos, 110);
    }

    #[test]
    fn sync_also_resolves_when_spindle_reverses() {
        let mut g = engaged_1mm();
        g.toggle_stop(Side::Left, 0, 100);
        g.update(7200);
        g.toggle_stop(Side::Left, 7200, 100);
        assert_eq!(g.update(6100).pos, 100);
        assert!(g.syncing());
        // Crossing 6000 (= 1200 mod 2400) re-syncs and follows backwards.
        assert_eq!(g.update(5880).pos, 90);
        assert!(!g.syncing());
    }

    #[test]
    fn right_stop_clamps() {
        let mut g = engaged_1mm();
        g.toggle_stop(Side::Right, 0, -50);
        assert_eq!(g.update(-24_000), Target::whole(-50, true));
        // Overshoot was trimmed to 1800 counts; 1200 counts forward is still on the stop.
        assert_eq!(g.update(-24_000 + 1200).pos, -50);
        assert_eq!(g.update(-24_000 + 2400).pos, 0);
    }

    #[test]
    fn pitch_change_while_engaged_does_not_jump() {
        let mut g = engaged_1mm();
        assert_eq!(g.update(2400).pos, 200);
        g.set_pitch_du(20_000, 2400);
        assert_eq!(g.update(2400).pos, 200);
        assert_eq!(g.update(4800).pos, 600);
        g.set_pitch_du(-20_000, 4800);
        assert_eq!(g.update(7200).pos, 200);
    }

    #[test]
    fn long_runs_do_not_overflow_or_drift() {
        let mut g = engaged_1mm();
        let s = 2400 * 10_000_000i64; // ten million turns
        assert_eq!(g.update(s).pos, 200 * 10_000_000);
        assert_eq!(g.update(s + 1200).pos, 200 * 10_000_000 + 100);
    }

    #[test]
    fn inch_pitch_ratio() {
        let mut g = Gearbox::new(M);
        g.set_pitch_du(254_000, 0); // 1" per rev = 25.4mm = 5080 steps
        g.engage(0, 0);
        assert_eq!(g.update(2400).pos, 5080);
    }
}
