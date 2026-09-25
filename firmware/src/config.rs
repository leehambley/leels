//! Everything specific to your machine and board. Edit to suit your hardware.

use els_core::jog::{JogConfig, JOG_LEVELS};
use els_core::stepgen::{StepGenConfig, MAX_STEPS_PER_SEGMENT};
use els_core::Machine;

// ---------------------------------------------------------------------------
// Pinout
// ---------------------------------------------------------------------------

/// Board pins, taken out of `Peripherals` with `take_pins!`.
pub struct Pins {
    pub encoder_a: esp_hal::gpio::AnyPin<'static>,
    pub encoder_b: esp_hal::gpio::AnyPin<'static>,
    pub step: esp_hal::gpio::AnyPin<'static>,
    pub dir: esp_hal::gpio::AnyPin<'static>,
    pub enable: esp_hal::gpio::AnyPin<'static>,
    pub nextion_tx: esp_hal::gpio::AnyPin<'static>,
    pub nextion_rx: esp_hal::gpio::AnyPin<'static>,
}

/// ESP32-C6 (primary target). Avoid strapping pins 8, 9, 15, USB 12/13 and
/// flash 24-30.
#[cfg(feature = "esp32c6")]
macro_rules! take_pins {
    ($p:ident) => {
        $crate::config::Pins {
            encoder_a: esp_hal::gpio::Pin::degrade($p.GPIO2),
            encoder_b: esp_hal::gpio::Pin::degrade($p.GPIO3),
            step: esp_hal::gpio::Pin::degrade($p.GPIO4),
            dir: esp_hal::gpio::Pin::degrade($p.GPIO5),
            enable: esp_hal::gpio::Pin::degrade($p.GPIO6),
            nextion_tx: esp_hal::gpio::Pin::degrade($p.GPIO22),
            nextion_rx: esp_hal::gpio::Pin::degrade($p.GPIO23),
        }
    };
}

/// ESP32-S3 on the NanoEls H5 board (secondary target, needs the Xtensa toolchain).
#[cfg(feature = "esp32s3")]
macro_rules! take_pins {
    ($p:ident) => {
        $crate::config::Pins {
            encoder_a: esp_hal::gpio::Pin::degrade($p.GPIO13),
            encoder_b: esp_hal::gpio::Pin::degrade($p.GPIO14),
            step: esp_hal::gpio::Pin::degrade($p.GPIO35),
            dir: esp_hal::gpio::Pin::degrade($p.GPIO42),
            enable: esp_hal::gpio::Pin::degrade($p.GPIO41),
            nextion_tx: esp_hal::gpio::Pin::degrade($p.GPIO43),
            nextion_rx: esp_hal::gpio::Pin::degrade($p.GPIO44),
        }
    };
}

// ---------------------------------------------------------------------------
// Spindle encoder
// ---------------------------------------------------------------------------

/// Encoder lines per revolution. Two counts are taken per line.
pub const ENCODER_PPR: i64 = 1200;
/// Counts the spindle may reverse without the carriage following (encoder jitter).
pub const ENCODER_BACKLASH: i64 = 3;
/// Flip if the carriage runs the wrong way for a positive pitch (or swap A/B).
pub const INVERT_SPINDLE: bool = false;
/// Glitch filter on the encoder inputs, in APB clock cycles (80MHz): 40 = 0.5 µs.
pub const ENCODER_FILTER_CYCLES: u16 = 40;

// ---------------------------------------------------------------------------
// Lead screw and stepper
// ---------------------------------------------------------------------------

/// Lead screw pitch in deci-microns (40000 = 4mm, e.g. SFU1204).
pub const SCREW_DU: i64 = 40_000;
/// Motor steps per lead screw turn: full steps * microsteps * gear ratio.
pub const MOTOR_STEPS: i64 = 800;
/// Speed the motor can start and stop at without ramping, steps/s.
pub const SPEED_START: u32 = 800;
/// steps/s².
pub const ACCELERATION: u32 = 25 * 800;
/// steps/s. Limited by the motor and driver, not the firmware.
pub const SPEED_MAX: u32 = 30_000;

/// Flip if positive pitch drives the carriage the wrong way.
pub const INVERT_DIR: bool = false;
/// Flip if the driver's enable input is active-low.
pub const INVERT_ENABLE: bool = false;
/// STEP idles high and pulses low, as on the NanoEls H5.
pub const STEP_ACTIVE_LOW: bool = true;
/// STEP pulse width, ns (driver datasheet minimum, with margin).
pub const STEP_PULSE_NS: u32 = 2_500;
/// DIR must be stable this long before a STEP edge, ns.
pub const DIR_SETUP_NS: u32 = 10_000;

// ---------------------------------------------------------------------------
// Motion timing
// ---------------------------------------------------------------------------

/// Control period: the spindle is sampled and one segment of pulses planned per period.
pub const SEGMENT_US: u32 = 250;
/// RMT clock: 80MHz source / 8 = 10MHz, i.e. 100ns pulse timing resolution.
pub const RMT_SOURCE_MHZ: u32 = 80;
pub const RMT_DIVIDER: u8 = 8;
pub const RMT_TICK_HZ: u32 = RMT_SOURCE_MHZ * 1_000_000 / RMT_DIVIDER as u32;
/// Spindle velocity is estimated over this many segments, for prediction.
pub const SPINDLE_VELOCITY_SEGMENTS: usize = 8;
/// How often the motion task publishes status for the display, in segments.
pub const STATUS_EVERY_SEGMENTS: u32 = 80;
/// RPM averaging window.
pub const RPM_WINDOW_US: u64 = 250_000;

// ---------------------------------------------------------------------------
// Jogging
// ---------------------------------------------------------------------------

/// Hold a jog button this long to switch from a single move to continuous motion.
pub const JOG_HOLD_AFTER_MS: u64 = 400;
/// While holding, step up to the next speed this often.
pub const JOG_LEVEL_EVERY_MS: u64 = 1_000;
/// Jog speed levels in du/s (mm/s * 10000), slowest first. Taps use the last.
pub const JOG_SPEEDS_DU_PER_S: [i64; JOG_LEVELS] = [5_000, 20_000, 80_000, 250_000]; // 0.5, 2, 8, 25 mm/s

// ---------------------------------------------------------------------------
// Operator interface
// ---------------------------------------------------------------------------

/// Largest pitch accepted, in deci-microns (254000 = 1").
pub const MAX_PITCH_DU: i64 = 254_000;
pub const NEXTION_BAUD: u32 = 115_200;
/// Nextion needs time to boot before it accepts commands.
pub const NEXTION_BOOT_MS: u64 = 1_300;
pub const DISPLAY_REFRESH_MS: u64 = 100;
/// Resend every field periodically in case the display was power-cycled.
pub const DISPLAY_FULL_REFRESH_MS: u64 = 5_000;

// ---------------------------------------------------------------------------
// Derived values and sanity checks
// ---------------------------------------------------------------------------

pub const fn machine() -> Machine {
    Machine { counts_per_rev: ENCODER_PPR * 2, motor_steps: MOTOR_STEPS, screw_du: SCREW_DU }
}

pub const fn stepgen() -> StepGenConfig {
    StepGenConfig {
        tick_hz: RMT_TICK_HZ,
        segment_ticks: SEGMENT_US * (RMT_TICK_HZ / 1_000_000),
        pulse_ticks: ns_to_ticks(STEP_PULSE_NS),
        dir_setup_ticks: ns_to_ticks(DIR_SETUP_NS),
        start_speed: SPEED_START,
        max_speed: SPEED_MAX,
        acceleration: ACCELERATION,
    }
}

pub fn jog() -> JogConfig {
    let m = machine();
    JogConfig {
        hold_after_us: JOG_HOLD_AFTER_MS * 1_000,
        level_every_us: JOG_LEVEL_EVERY_MS * 1_000,
        speeds: JOG_SPEEDS_DU_PER_S.map(|du| m.du_to_steps(du).clamp(1, SPEED_MAX as i64) as u32),
    }
}

const fn ns_to_ticks(ns: u32) -> u32 {
    (ns as u64 * RMT_TICK_HZ as u64).div_ceil(1_000_000_000) as u32
}

const _: () = {
    let g = stepgen();
    assert!(SPEED_MAX <= g.pulse_limited_speed(), "STEP pulse too wide for SPEED_MAX");
    assert!(g.max_steps_per_segment() <= MAX_STEPS_PER_SEGMENT as u64, "SEGMENT_US too long for SPEED_MAX");
    assert!(g.segment_ticks <= 32_767, "a segment must fit one RMT item length");
    assert!(g.dir_setup_ticks < g.segment_ticks / 2);
    assert!(SPEED_START <= SPEED_MAX);
};
