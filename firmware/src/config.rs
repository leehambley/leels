//! Everything specific to your board, plus the machine defaults. Machine
//! settings can also be changed at runtime from the setup web page.

use els_core::settings::{Settings, Timing};

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
// Machine defaults
//
// Used until settings are saved from the setup web page (Setup button on the
// display), and whenever saved settings are missing or damaged.
// ---------------------------------------------------------------------------

pub const DEFAULTS: Settings = Settings {
    // Spindle encoder: lines per revolution (2 counts per line), counts the
    // spindle may reverse without the carriage following, glitch filter in
    // 12.5 ns APB cycles (40 = 0.5 µs), direction.
    encoder_ppr: 1200,
    encoder_backlash: 3,
    encoder_filter_cycles: 40,
    invert_spindle: false,
    // Lead screw pitch in deci-microns (40000 = 4mm, e.g. SFU1204); motor
    // steps per screw turn (full steps * microsteps * gear ratio).
    screw_du: 40_000,
    motor_steps: 800,
    // steps/s, steps/s, steps/s².
    speed_start: 800,
    speed_max: 30_000,
    acceleration: 20_000,
    // Largest pitch accepted, deci-microns (254000 = 1").
    max_pitch_du: 254_000,
    // Driver signals. STEP idles high and pulses low on the NanoEls H5.
    invert_dir: false,
    invert_enable: false,
    step_active_low: true,
    step_pulse_ns: 2_500,
    dir_setup_ns: 10_000,
    // Jog hold speeds in du/s (0.5, 2, 8, 25 mm/s); taps use the last one.
    jog_speeds_du_per_s: [5_000, 20_000, 80_000, 250_000],
    jog_hold_after_ms: 400,
    jog_level_every_ms: 1_000,
};

// ---------------------------------------------------------------------------
// Motion timing (fixed in firmware)
// ---------------------------------------------------------------------------

/// Control period: the spindle is sampled and one segment of pulses planned per period.
pub const SEGMENT_US: u32 = 250;
/// RMT clock: 80MHz source / 8 = 10MHz, i.e. 100ns pulse timing resolution.
pub const RMT_SOURCE_MHZ: u32 = 80;
pub const RMT_DIVIDER: u8 = 8;
pub const RMT_TICK_HZ: u32 = RMT_SOURCE_MHZ * 1_000_000 / RMT_DIVIDER as u32;
pub const TIMING: Timing = Timing { tick_hz: RMT_TICK_HZ, segment_ticks: SEGMENT_US * (RMT_TICK_HZ / 1_000_000) };
/// Spindle velocity is estimated over this many segments, for prediction.
pub const SPINDLE_VELOCITY_SEGMENTS: usize = 8;
/// How often the motion task publishes status for the display, in segments.
pub const STATUS_EVERY_SEGMENTS: u32 = 80;
/// RPM averaging window.
pub const RPM_WINDOW_US: u64 = 250_000;

const _: () = assert!(TIMING.segment_ticks <= 32_767, "a segment must fit one RMT item length");

// ---------------------------------------------------------------------------
// Operator interface
// ---------------------------------------------------------------------------

pub const NEXTION_BAUD: u32 = 115_200;
/// Nextion needs time to boot before it accepts commands.
pub const NEXTION_BOOT_MS: u64 = 1_300;
pub const DISPLAY_REFRESH_MS: u64 = 100;
/// Resend every field periodically in case the display was power-cycled.
pub const DISPLAY_FULL_REFRESH_MS: u64 = 5_000;
/// Pitch/unit/step choices are saved this long after the last change, once
/// the carriage is idle (flash writes pause the CPU for tens of ms).
pub const STATE_SAVE_DELAY_MS: u64 = 3_000;

// ---------------------------------------------------------------------------
// Setup hotspot
// ---------------------------------------------------------------------------

pub const SETUP_SSID: &str = "LEELS-SETUP";
/// WPA2 password, 8-63 characters. Shown on the display in setup mode.
pub const SETUP_PASSWORD: &str = "els-setup";
pub const SETUP_IP: [u8; 4] = [192, 168, 4, 1];
pub const SETUP_URL: &str = "http://192.168.4.1/";
/// Message line while the hotspot is up (fits the 32-character field).
pub const SETUP_BANNER: &str = "WiFi LEELS-SETUP pw els-setup";
