//! Step pulse scheduling: follows a moving target with a speed and
//! acceleration limit. Polled from a fixed-rate motion tick.

use crate::gearbox::Target;

/// Time the driver needs between a DIR change and the next STEP edge.
pub const DIR_SETUP_US: u64 = 10;

#[derive(Clone, Copy, Debug)]
pub struct StepperConfig {
    /// Speed a move starts from, steps/s.
    pub start_speed: u32,
    /// Speed cap, steps/s.
    pub max_speed: u32,
    /// steps/s^2.
    pub acceleration: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Idle,
    /// Set the DIR pin (`true` = positive direction). No step this tick.
    SetDirection(bool),
    /// Emit one STEP pulse.
    Step,
}

pub struct Stepper {
    cfg: StepperConfig,
    /// Motor position in steps, counting pulses already issued.
    pub pos: i64,
    dir: Option<bool>,
    speed: u32,
    speed_cap: Option<u32>,
    rem: u32,
    next_due_us: u64,
    last_step_us: Option<u64>,
}

impl Stepper {
    pub fn new(cfg: StepperConfig) -> Self {
        let start_speed = cfg.start_speed.max(1);
        let cfg = StepperConfig { start_speed, max_speed: cfg.max_speed.max(start_speed), ..cfg };
        Self { cfg, pos: 0, dir: None, speed: start_speed, speed_cap: None, rem: 0, next_due_us: 0, last_step_us: None }
    }

    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// Temporarily limit the top speed below `max_speed` (used by jogging).
    pub fn set_speed_cap(&mut self, cap: Option<u32>) {
        self.speed_cap = cap;
    }

    /// Start speed, lowered by the speed cap if that is slower.
    fn start_speed(&self) -> u32 {
        self.speed_cap.map_or(self.cfg.start_speed, |c| c.clamp(1, self.cfg.start_speed))
    }

    /// Steps needed to decelerate from the current speed to start speed.
    pub fn braking_steps(&self) -> i64 {
        let v = self.speed as u64;
        let v0 = self.start_speed() as u64;
        (v * v).saturating_sub(v0 * v0).div_ceil(2 * self.cfg.acceleration as u64) as i64
    }

    pub fn poll(&mut self, now_us: u64, target: Target) -> Action {
        let pending = target.pos - self.pos;
        if pending == 0 {
            return Action::Idle;
        }
        let positive = pending > 0;
        if self.dir != Some(positive) {
            self.dir = Some(positive);
            self.speed = self.start_speed();
            self.next_due_us = now_us + DIR_SETUP_US;
            return Action::SetDirection(positive);
        }
        if now_us < self.next_due_us {
            return Action::Idle;
        }

        // A long gap since the last pulse means the motor really slowed down.
        let interval = 1_000_000 / self.speed as u64;
        if let Some(last) = self.last_step_us {
            let gap = now_us - last;
            if gap > 2 * interval {
                let gap_speed = (1_000_000 / gap).max(1) as u32;
                self.speed = self.speed.min(gap_speed).max(self.start_speed());
            }
        }

        self.pos += if positive { 1 } else { -1 };
        self.last_step_us = Some(now_us);

        // v dv = a dx: per step the speed changes by a / v.
        let remaining = pending.unsigned_abs() - 1;
        let v = self.speed as u64;
        let braking = target.is_final && remaining * 2 * self.cfg.acceleration as u64 <= v * v;
        let total = self.cfg.acceleration + self.rem;
        let dv = total / self.speed;
        self.rem = total % self.speed;
        self.speed = if braking {
            self.speed.saturating_sub(dv).max(self.start_speed())
        } else {
            let max = self.speed_cap.map_or(self.cfg.max_speed, |c| c.min(self.cfg.max_speed));
            (self.speed + dv).min(max.max(1))
        };

        // Schedule from the ideal due time so tick quantisation doesn't
        // lower the average rate; restart the schedule after idling.
        let interval = 1_000_000 / self.speed as u64;
        let base = if now_us - self.next_due_us >= interval { now_us } else { self.next_due_us };
        self.next_due_us = base + interval;
        Action::Step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: StepperConfig = StepperConfig { start_speed: 800, max_speed: 6400, acceleration: 20_000 };

    fn target(pos: i64, is_final: bool) -> Target {
        Target { pos, is_final }
    }

    /// Run with a 20us tick until the target is reached; returns step times.
    fn run(s: &mut Stepper, t: Target, mut now: u64) -> Vec<u64> {
        let mut steps = Vec::new();
        while s.pos != t.pos {
            if s.poll(now, t) == Action::Step {
                steps.push(now);
            }
            now += 20;
            assert!(now < 10_000_000, "did not converge");
        }
        steps
    }

    #[test]
    fn sets_direction_before_stepping() {
        let mut s = Stepper::new(CFG);
        assert_eq!(s.poll(0, target(5, false)), Action::SetDirection(true));
        assert_eq!(s.poll(0, target(5, false)), Action::Idle);
        assert_eq!(s.poll(20, target(5, false)), Action::Step);
        assert_eq!(s.pos, 1);
        assert_eq!(s.poll(40, target(-5, false)), Action::SetDirection(false));
    }

    #[test]
    fn accelerates_to_max_and_brakes_on_final_target() {
        let mut s = Stepper::new(CFG);
        let steps = run(&mut s, target(4000, true), 0);
        assert_eq!(steps.len(), 4000);
        // Peak speed reached somewhere in the middle.
        let min_gap = steps.windows(2).map(|w| w[1] - w[0]).min().unwrap();
        assert!(min_gap <= 180, "min gap {min_gap}");
        // Last steps are slow again.
        let last_gap = steps[3999] - steps[3998];
        assert!(last_gap >= 900, "last gap {last_gap}");
    }

    #[test]
    fn average_rate_not_limited_by_tick() {
        // 6400 steps/s is a 156us interval, not a multiple of the 20us tick.
        let mut s = Stepper::new(StepperConfig { start_speed: 6400, max_speed: 6400, acceleration: 1 });
        let steps = run(&mut s, target(6401, false), 0);
        let elapsed = steps[6400] - steps[0];
        assert!((990_000..=1_010_000).contains(&elapsed), "elapsed {elapsed}");
    }

    #[test]
    fn speed_cap_limits_and_braking_distance() {
        let mut s = Stepper::new(CFG);
        s.set_speed_cap(Some(2000));
        let steps = run(&mut s, target(3000, false), 0);
        let min_gap = steps.windows(2).map(|w| w[1] - w[0]).min().unwrap();
        assert!(min_gap >= 480, "min gap {min_gap}");
        // (2000^2 - 800^2) / (2 * 20000) = 84
        assert_eq!(s.braking_steps(), 84);
    }

    #[test]
    fn cap_below_start_speed_runs_slower() {
        let mut s = Stepper::new(CFG);
        s.set_speed_cap(Some(100));
        let steps = run(&mut s, target(11, false), 0);
        assert_eq!(steps[10] - steps[0], 100_000);
    }

    #[test]
    fn idle_gap_resets_speed() {
        let mut s = Stepper::new(CFG);
        run(&mut s, target(2000, false), 0);
        assert!(s.speed() > 5000);
        // Pause, then continue: must restart near start speed.
        s.poll(5_000_000, target(2010, false));
        assert!(s.speed() < 1000, "speed {}", s.speed());
    }
}
