//! Segment step generator.
//!
//! The control loop runs once per segment (e.g. 250 µs). Each call to
//! [`StepGen::next_segment`] takes the position the motor should reach at the
//! end of the segment and returns the exact tick of every STEP pulse inside
//! it, for a hardware pulse sequencer (RMT) to play back.
//!
//! Within a segment the velocity is constant, so pulses are evenly spaced.
//! Between segments the velocity changes by at most `acceleration`, except
//! that anything up to `start_speed` may be jumped to directly.
//!
//! Tracking law: velocity = target velocity (feed-forward, from how far a
//! moving target advanced since the last segment) + a correction that closes
//! the remaining error in one segment but no faster than it could still be
//! braked away (`√(v₀² + 2·a·|error|)`). This follows a moving target
//! without lag, and brakes into a final target without overshoot.
//!
//! Position is fixed point: 32 fractional bits of a step. A step is emitted
//! when the commanded position crosses the half-way point between two
//! steps, so the motor position is always `round(commanded)`.

use crate::gearbox::Target;

const ONE: i64 = 1 << 32;
const HALF: i64 = 1 << 31;

/// Capacity of a segment; the firmware checks the configuration against it.
pub const MAX_STEPS_PER_SEGMENT: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct StepGenConfig {
    /// Pulse sequencer clock, ticks per second.
    pub tick_hz: u32,
    /// Segment length in ticks.
    pub segment_ticks: u32,
    /// STEP pulse width in ticks.
    pub pulse_ticks: u32,
    /// Minimum time between a DIR change and the next STEP edge, in ticks.
    pub dir_setup_ticks: u32,
    /// Speed that can be started from or stopped at instantly, steps/s.
    pub start_speed: u32,
    /// Speed limit, steps/s.
    pub max_speed: u32,
    /// steps/s².
    pub acceleration: u32,
}

impl StepGenConfig {
    /// Highest speed the pulse timing allows: one pulse plus one idle tick per step.
    pub const fn pulse_limited_speed(&self) -> u32 {
        self.tick_hz / (self.pulse_ticks + 1)
    }

    /// Largest number of steps `max_speed` can produce in one segment.
    pub const fn max_steps_per_segment(&self) -> u64 {
        (self.max_speed as u64 * self.segment_ticks as u64).div_ceil(self.tick_hz as u64) + 1
    }
}

/// One segment of pulses to play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    /// DIR level to set before playing (`true` = positive).
    pub positive: bool,
    /// Rising-edge tick of each STEP pulse, from segment start, ascending.
    pub steps: heapless::Vec<u32, MAX_STEPS_PER_SEGMENT>,
}

/// One pulse-sequencer item: `idle` ticks inactive, then `pulse` ticks active.
/// The last item of a segment has `pulse == 0` and marks the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub idle: u16,
    pub pulse: u16,
}

impl Segment {
    /// Encode as sequencer items covering exactly `segment_ticks`.
    pub fn items(&self, cfg: &StepGenConfig) -> heapless::Vec<Item, { MAX_STEPS_PER_SEGMENT + 1 }> {
        let mut out = heapless::Vec::new();
        let mut t = 0u32;
        for &at in &self.steps {
            let _ = out.push(Item { idle: (at - t) as u16, pulse: cfg.pulse_ticks as u16 });
            t = at + cfg.pulse_ticks;
        }
        let _ = out.push(Item { idle: (cfg.segment_ticks - t) as u16, pulse: 0 });
        out
    }
}

pub struct StepGen {
    cfg: StepGenConfig,
    /// Commanded position, fixed point steps.
    p: i64,
    /// Velocity, fixed point steps per tick.
    v: i64,
    /// Steps emitted so far (the motor position).
    issued: i64,
    positive: bool,
    speed_cap: Option<u32>,
    dv_per_segment: i64,
    /// Previous non-final target, fixed point, for feed-forward.
    prev_target: Option<i64>,
}

impl StepGen {
    pub fn new(cfg: StepGenConfig) -> Self {
        let dv = cfg.acceleration as i128 * cfg.segment_ticks as i128 * ONE as i128
            / (cfg.tick_hz as i128 * cfg.tick_hz as i128);
        Self {
            cfg,
            p: 0,
            v: 0,
            issued: 0,
            positive: true,
            speed_cap: None,
            dv_per_segment: (dv as i64).max(1),
            prev_target: None,
        }
    }

    /// Motor position in steps (pulses issued).
    pub fn pos(&self) -> i64 {
        self.issued
    }

    /// Current speed, steps/s.
    pub fn speed(&self) -> u32 {
        (self.v.unsigned_abs() as u128 * self.cfg.tick_hz as u128 / ONE as u128) as u32
    }

    /// Temporarily limit the speed (used by jogging). May be below start speed.
    pub fn set_speed_cap(&mut self, cap: Option<u32>) {
        self.speed_cap = cap;
    }

    /// Steps needed to brake from the current speed.
    pub fn braking_steps(&self) -> i64 {
        let v = self.speed() as u64;
        let v0 = self.cfg.start_speed as u64;
        (v * v).saturating_sub(v0 * v0).div_ceil(2 * self.cfg.acceleration.max(1) as u64) as i64
    }

    /// Plan the next segment so the motor is at `target.pos` at its end, within
    /// the speed and acceleration limits.
    pub fn next_segment(&mut self, target: Target) -> Segment {
        let t = self.cfg.segment_ticks as i64;
        let tgt = (target.pos << 32) + target.frac as i64;

        let mut v_max = self.to_v(self.cfg.max_speed);
        if let Some(cap) = self.speed_cap {
            v_max = v_max.min(self.to_v(cap).max(1));
        }

        // Feed-forward: how fast a moving target is going.
        let v_ff = match (target.is_final, self.prev_target) {
            (false, Some(prev)) => ((tgt - prev) / t).clamp(-v_max, v_max),
            _ => 0,
        };
        self.prev_target = (!target.is_final).then_some(tgt);

        // Correction for what feed-forward alone would leave, limited so it
        // could still be braked away.
        // Relative to a moving target only acceleration counts; a final
        // target can additionally be stopped at from start speed.
        let e = tgt - (self.p + v_ff * t);
        let v0 = if target.is_final { self.cfg.start_speed as u128 } else { 0 };
        let two_a_d = 2 * self.cfg.acceleration as u128 * e.unsigned_abs() as u128; // (steps/s)² · 2^32
        let brake_sps = isqrt(((v0 * v0) << 32) + two_a_d) >> 16; // steps/s · 2^16 → steps/s
        let brake = self.to_v(brake_sps.min(u32::MAX as u128) as u32);
        let correction = (e / t).clamp(-brake, brake);
        let desired = (v_ff + correction).clamp(-v_max, v_max);

        // Reachable: within one segment's acceleration of now, or anything
        // up to start speed. Take the reachable value closest to `desired`.
        let v_start = self.to_v(self.cfg.start_speed).min(v_max);
        let accel = desired.clamp(self.v - self.dv_per_segment, self.v + self.dv_per_segment);
        let jump = desired.clamp(-v_start, v_start);
        let mut v = if (accel - desired).abs() <= (jump - desired).abs() { accel } else { jump };

        let err = tgt - self.p;
        let mut p1 = self.p + v * t;
        if target.is_final && (err > 0 && p1 > tgt || err < 0 && p1 < tgt) {
            p1 = tgt;
            v = err / t;
        }
        let p0 = self.p;
        self.p = p1;
        self.v = v;
        self.emit(p0, round(p1) - self.issued)
    }

    fn emit(&mut self, p0: i64, want: i64) -> Segment {
        let mut seg = Segment { positive: self.positive, steps: heapless::Vec::new() };
        if want == 0 {
            return seg;
        }
        let positive = want > 0;
        // Item lengths must be non-zero, so nothing starts at tick 0.
        let mut earliest = 1i64;
        if positive != self.positive {
            self.positive = positive;
            seg.positive = positive;
            earliest = earliest.max(self.cfg.dir_setup_ticks as i64);
        }
        let latest = self.cfg.segment_ticks as i64 - self.cfg.pulse_ticks as i64 - 1;
        let gap = self.cfg.pulse_ticks as i64 + 1;
        let v = self.v;
        for _ in 0..want.unsigned_abs() {
            let boundary = if positive { (self.issued << 32) + HALF } else { (self.issued << 32) - HALF };
            let dist = boundary - p0;
            let crossing = if v != 0 && (dist > 0) == (v > 0) {
                (dist.abs() + v.abs() - 1) / v.abs()
            } else {
                0 // already crossed, owed from the previous segment
            };
            let at = crossing.max(earliest);
            if at > latest || seg.steps.push(at as u32).is_err() {
                break; // carried into the next segment
            }
            self.issued += if positive { 1 } else { -1 };
            earliest = at + gap;
        }
        seg
    }

    fn to_v(&self, steps_per_s: u32) -> i64 {
        (steps_per_s as i128 * ONE as i128 / self.cfg.tick_hz as i128) as i64
    }
}

fn round(p: i64) -> i64 {
    (p + HALF) >> 32
}

fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // Newton's method; no floating point on the C6.
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    // 10 MHz ticks, 250 µs segments, 2.5 µs pulses, 10 µs DIR setup.
    const CFG: StepGenConfig = StepGenConfig {
        tick_hz: 10_000_000,
        segment_ticks: 2_500,
        pulse_ticks: 25,
        dir_setup_ticks: 100,
        start_speed: 800,
        max_speed: 20_000,
        acceleration: 50_000,
    };

    /// Absolute step times (ticks) and signs, plus final position.
    fn run(g: &mut StepGen, segments: usize, target: impl Fn(usize) -> Target) -> Vec<(i64, i8)> {
        let mut out = Vec::new();
        for i in 0..segments {
            let seg = g.next_segment(target(i));
            let items = seg.items(&CFG);
            let total: u32 = items.iter().map(|i| i.idle as u32 + i.pulse as u32).sum();
            assert_eq!(total, CFG.segment_ticks, "segment {i} must fill exactly");
            assert!(items.iter().all(|i| i.idle > 0));
            for &s in &seg.steps {
                out.push((i as i64 * CFG.segment_ticks as i64 + s as i64, if seg.positive { 1 } else { -1 }));
            }
        }
        out
    }

    fn final_target(pos: i64) -> impl Fn(usize) -> Target {
        move |_| Target::whole(pos, true)
    }

    #[test]
    fn moves_to_final_target_without_overshoot() {
        let mut g = StepGen::new(CFG);
        let steps = run(&mut g, 4_000, final_target(5_000));
        assert_eq!(g.pos(), 5_000);
        assert!(steps.iter().all(|&(_, d)| d == 1), "no reversals");
        assert_eq!(steps.len(), 5_000);
        assert_eq!(g.speed(), 0);
    }

    #[test]
    fn respects_max_speed_and_pulse_gap() {
        let mut g = StepGen::new(CFG);
        let steps = run(&mut g, 2_000, final_target(20_000));
        let min_interval = steps.windows(2).map(|w| w[1].0 - w[0].0).min().unwrap();
        // 20k steps/s = 500 ticks; allow one tick of rounding.
        assert!(min_interval >= 497, "min interval {min_interval}");
        assert!(min_interval > CFG.pulse_ticks as i64);
    }

    /// Spindle-like target: ramps from rest to `v_end` steps/s over `ramp`
    /// segments, then runs steadily. Fixed point positions per segment.
    fn ramp_target(v_end: i64, ramp: usize) -> impl Fn(usize) -> Target {
        move |i| {
            let seg = (1i64 << 32) / 4_000; // one 250 µs segment, fixed point seconds
            let n = (i + 1) as i64;
            let r = ramp as i64;
            let fp = if n <= r { v_end * n * n / (2 * r) * seg } else { (v_end * r / 2 + v_end * (n - r)) * seg };
            Target { pos: fp >> 32, frac: fp as u32, is_final: false }
        }
    }

    #[test]
    fn follows_constant_velocity_evenly_across_segments() {
        // Spindle spins up over 0.5 s, then 6,000 steps/s = 1.5 steps per segment.
        let mut g = StepGen::new(CFG);
        let steps = run(&mut g, 6_000, ramp_target(6_000, 2_000));
        let settled = 2_500 * CFG.segment_ticks as i64;
        let late: Vec<i64> = steps.windows(2).filter(|w| w[0].0 > settled).map(|w| w[1].0 - w[0].0).collect();
        let (min, max) = (*late.iter().min().unwrap(), *late.iter().max().unwrap());
        // Ideal interval is 1666.67 ticks (166.7 µs).
        assert!(min >= 1_665 && max <= 1_668, "interval spread {min}..{max}");
    }

    #[test]
    fn tracks_spindle_acceleration_without_lag() {
        // 0 → 8,000 steps/s over 1 s (well within the 50,000 steps/s² limit).
        let mut g = StepGen::new(CFG);
        let target = ramp_target(8_000, 4_000);
        let mut worst = 0i64;
        for i in 0..8_000usize {
            let t = target(i);
            g.next_segment(t);
            worst = worst.max((t.pos - g.pos()).abs());
        }
        assert!(worst <= 1, "lag {worst} steps");
    }

    #[test]
    fn reversal_waits_for_dir_setup() {
        let mut g = StepGen::new(CFG);
        run(&mut g, 200, final_target(300));
        let mut first_back = None;
        for _ in 0..200 {
            let seg = g.next_segment(Target::whole(0, true));
            if !seg.positive && !seg.steps.is_empty() {
                first_back = Some(seg.steps[0]);
                break;
            }
        }
        assert!(first_back.unwrap() >= CFG.dir_setup_ticks);
    }

    #[test]
    fn speed_cap_below_start_speed() {
        let mut g = StepGen::new(CFG);
        g.set_speed_cap(Some(100));
        let steps = run(&mut g, 4_000, final_target(1_000_000));
        // 100 steps/s over 1 s of segments.
        assert!((99..=101).contains(&steps.len()), "{} steps", steps.len());
    }

    #[test]
    fn acceleration_is_limited() {
        let mut g = StepGen::new(CFG);
        let mut last = 0u32;
        for _ in 0..400 {
            g.next_segment(Target::whole(1_000_000, false));
            let s = g.speed();
            // dv per segment = 50000 * 250e-6 = 12.5 steps/s, plus start jump.
            assert!(s <= last.max(CFG.start_speed) + 13, "{last} -> {s}");
            last = s;
        }
    }

    #[test]
    fn braking_steps_estimate() {
        let mut g = StepGen::new(CFG);
        run(&mut g, 4_000, |_| Target::whole(10_000_000, false));
        assert!((19_999..=20_000).contains(&g.speed()));
        // (20000² - 800²) / (2 * 50000) = 3994
        assert!((3_993..=3_994).contains(&g.braking_steps()));
    }
}
