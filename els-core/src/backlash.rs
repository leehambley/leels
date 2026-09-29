//! Lead screw backlash compensation.
//!
//! When the carriage reverses, the screw turns through the slack in the nut
//! before the carriage moves. So on every reversal the motor is sent that
//! many steps further than the carriage: while moving towards -Z the motor
//! runs `steps` behind (more negative than) the carriage position, while
//! moving towards +Z it matches.
//!
//! Everything else (gearbox, jog, stops, the Z readout) works in carriage
//! positions; only the step generator sees motor positions.

use crate::gearbox::Target;

#[derive(Clone, Copy, Debug)]
pub struct Backlash {
    steps: i64,
    /// Last direction the carriage was moved in. Assumed +Z at power-up:
    /// where the slack sits then is unknown.
    positive: bool,
    /// Last whole-step carriage target, to spot reversals.
    last: Option<i64>,
}

impl Backlash {
    pub fn new(steps: i64) -> Self {
        Self { steps: steps.max(0), positive: true, last: None }
    }

    /// Change the amount (backlash wizard). While moving towards -Z the
    /// motor then moves by the difference; the carriage doesn't.
    pub fn set_steps(&mut self, steps: i64) {
        self.steps = steps.max(0);
    }

    /// Motor minus carriage position.
    fn offset(&self) -> i64 {
        if self.positive {
            0
        } else {
            -self.steps
        }
    }

    /// Motor target for a carriage target. A reversal is a move of at least
    /// one whole step against the last direction, so sub-step jitter in a
    /// gearbox target never swaps the slack back and forth.
    pub fn motor_target(&mut self, t: Target) -> Target {
        if let Some(last) = self.last {
            if t.pos > last {
                self.positive = true;
            } else if t.pos < last {
                self.positive = false;
            }
        }
        self.last = Some(t.pos);
        Target { pos: t.pos + self.offset(), ..t }
    }

    /// Carriage position for a motor position.
    pub fn carriage(&self, motor: i64) -> i64 {
        motor - self.offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stepgen::{StepGen, StepGenConfig};

    const CFG: StepGenConfig = StepGenConfig {
        tick_hz: 10_000_000,
        segment_ticks: 2_500,
        pulse_ticks: 25,
        dir_setup_ticks: 100,
        start_speed: 800,
        max_speed: 20_000,
        acceleration: 50_000,
    };

    /// Drive the motor to a carriage target and return the motor steps taken.
    fn move_to(g: &mut StepGen, b: &mut Backlash, carriage: i64) -> i64 {
        let start = g.pos();
        for _ in 0..4_000 {
            g.next_segment(b.motor_target(Target::whole(carriage, true)));
        }
        assert_eq!(b.carriage(g.pos()), carriage);
        g.pos() - start
    }

    #[test]
    fn reversals_take_up_the_slack_once() {
        let (mut g, mut b) = (StepGen::new(CFG), Backlash::new(30));
        assert_eq!(move_to(&mut g, &mut b, 100), 100);
        // Reversing: 30 extra motor steps, carriage lands on target.
        assert_eq!(move_to(&mut g, &mut b, 0), -130);
        // Same direction again: no extra.
        assert_eq!(move_to(&mut g, &mut b, -50), -50);
        // Back towards +Z: the slack is taken up the other way.
        assert_eq!(move_to(&mut g, &mut b, 0), 80);
    }

    #[test]
    fn holding_still_or_sub_step_jitter_keeps_the_direction() {
        let mut b = Backlash::new(30);
        b.motor_target(Target::whole(10, true));
        b.motor_target(Target::whole(5, true));
        assert_eq!(b.carriage(-25), 5);
        // Same whole step, only the fraction wobbling: still -Z.
        let t = b.motor_target(Target { pos: 5, frac: u32::MAX, is_final: false });
        assert_eq!((t.pos, t.frac), (-25, u32::MAX));
        b.motor_target(Target::whole(5, true));
        assert_eq!(b.carriage(-25), 5);
    }

    #[test]
    fn changing_the_amount_moves_only_the_motor() {
        let (mut g, mut b) = (StepGen::new(CFG), Backlash::new(0));
        move_to(&mut g, &mut b, 100);
        move_to(&mut g, &mut b, 0);
        assert_eq!(g.pos(), 0);
        b.set_steps(40);
        assert_eq!(move_to(&mut g, &mut b, 0), -40);
        b.set_steps(-5);
        assert_eq!(move_to(&mut g, &mut b, 0), 40);
    }
}
