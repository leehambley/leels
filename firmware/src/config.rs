//! Machine configuration. Edit to suit your hardware.

use els_core::jog::{JogConfig, JOG_LEVELS};
use els_core::stepper::StepperConfig;
use els_core::Machine;

/// Spindle encoder lines per revolution. Two counts are taken per line.
pub const ENCODER_PPR: i64 = 1200;
/// Counts the spindle may reverse without the carriage following.
pub const ENCODER_BACKLASH: i64 = 3;
/// Flip if the carriage runs the wrong way for a positive pitch (or swap A/B).
pub const INVERT_SPINDLE: bool = false;
/// Glitch filter on the encoder inputs, in APB clock cycles (80MHz).
pub const ENCODER_FILTER_CYCLES: u16 = 40;

/// Lead screw pitch in deci-microns (40000 = 4mm, e.g. SFU1204).
pub const SCREW_DU: i64 = 40_000;
/// Motor steps per lead screw turn: full steps * microsteps * gear ratio.
pub const MOTOR_STEPS: i64 = 800;
pub const SPEED_START: u32 = 800;
pub const ACCELERATION: u32 = 25 * 800;
/// Must not exceed 1s / (2 * MOTION_TICK_US): a step pulse lasts one tick.
pub const SPEED_MAX: u32 = 24_000;

/// Flip if positive pitch drives the carriage the wrong way.
pub const INVERT_DIR: bool = false;
/// Flip if the driver's enable input is active-low.
pub const INVERT_ENABLE: bool = false;
/// STEP idles high and pulses low, as on the NanoEls H5.
pub const STEP_ACTIVE_LOW: bool = true;

/// Hold a jog button this long to switch from a single step to continuous motion.
pub const JOG_HOLD_AFTER_MS: u64 = 400;
/// While holding, step up to the next speed this often.
pub const JOG_LEVEL_EVERY_MS: u64 = 1_000;
/// Jog speed levels in mm/s * 10000 (du/s), slowest first. Taps use the last.
pub const JOG_SPEEDS_DU_PER_S: [i64; JOG_LEVELS] = [5_000, 20_000, 80_000, 250_000]; // 0.5, 2, 8, 25 mm/s

/// Largest pitch accepted, in deci-microns (254000 = 1").
pub const MAX_PITCH_DU: i64 = 254_000;

/// Motion loop period. Each tick samples the spindle and may issue one edge.
pub const MOTION_TICK_US: u64 = 20;
/// How often the motion task publishes status for the display.
pub const STATUS_PERIOD_US: u64 = 20_000;
/// RPM averaging window.
pub const RPM_WINDOW_US: u64 = 250_000;

pub const NEXTION_BAUD: u32 = 115_200;
/// Nextion needs time to boot before it accepts commands.
pub const NEXTION_BOOT_MS: u64 = 1_300;
pub const DISPLAY_REFRESH_MS: u64 = 100;
/// Resend every field periodically in case the display was power-cycled.
pub const DISPLAY_FULL_REFRESH_MS: u64 = 5_000;

const _: () = assert!(SPEED_MAX as u64 <= 1_000_000 / (2 * MOTION_TICK_US));

pub const fn machine() -> Machine {
    Machine { counts_per_rev: ENCODER_PPR * 2, motor_steps: MOTOR_STEPS, screw_du: SCREW_DU }
}

pub fn jog() -> JogConfig {
    let m = machine();
    JogConfig {
        hold_after_us: JOG_HOLD_AFTER_MS * 1_000,
        level_every_us: JOG_LEVEL_EVERY_MS * 1_000,
        speeds: JOG_SPEEDS_DU_PER_S.map(|du| m.du_to_steps(du).clamp(1, SPEED_MAX as i64) as u32),
    }
}

pub const fn stepper() -> StepperConfig {
    StepperConfig { start_speed: SPEED_START, max_speed: SPEED_MAX, acceleration: ACCELERATION }
}
