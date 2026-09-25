//! Hardware-independent logic for a single-axis (Z) electronic gearbox.
//!
//! Everything in here is `no_std`, allocation-free and unit-tested on the host
//! with `cargo test`. The firmware crate only glues it to GPIO, PCNT, UART and
//! timers.
//!
//! Units used throughout:
//! - `du`: deci-microns, 10^-7 m (same as the original NanoEls firmware).
//! - `milli`: thousandths of the currently selected display unit (mm or inch).
//! - `counts`: spindle encoder counts (2 per encoder line, like the original).
//! - `steps`: stepper motor steps (including microsteps).
#![cfg_attr(not(test), no_std)]

pub mod display;
pub mod gearbox;
pub mod nextion;
pub mod pitch;
pub mod spindle;
pub mod stepper;
pub mod ui;

/// Fixed machine geometry needed to translate between spindle counts, motor
/// steps and physical distance.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// Encoder counts per spindle revolution (encoder PPR * 2).
    pub counts_per_rev: i64,
    /// Motor steps per lead screw revolution (full steps * microsteps * gearing).
    pub motor_steps: i64,
    /// Lead screw pitch in du.
    pub screw_du: i64,
}

impl Machine {
    pub fn steps_to_du(&self, steps: i64) -> i64 {
        mul_div_round(steps, self.screw_du, self.motor_steps)
    }
}

/// `a * b / c` rounded to nearest, without intermediate overflow.
pub(crate) fn mul_div_round(a: i64, b: i64, c: i64) -> i64 {
    let n = a as i128 * b as i128;
    let c = c as i128;
    let (q, r) = (n / c, n % c);
    let q = if 2 * r.abs() >= c.abs() { q + if (n < 0) == (c < 0) { 1 } else { -1 } } else { q };
    q as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_round_rounds_half_away() {
        assert_eq!(mul_div_round(5, 1, 2), 3);
        assert_eq!(mul_div_round(-5, 1, 2), -3);
        assert_eq!(mul_div_round(4, 1, 3), 1);
        assert_eq!(mul_div_round(i64::MAX / 2, 4, 4), i64::MAX / 2);
    }
}
