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

/// ESP32-C6-DevKitC-1 (primary target). Encoder A/B/Z and Nextion on J1/J3,
/// driver on J3 18-20. Avoid strapping pins 4, 5, 8, 9, 15, USB 12/13, the
/// console UART 16/17 and flash 24-30. GPIO7 is kept free for encoder Z
/// (index), which the firmware doesn't read yet. GPIO2/3 took 5 V pull-ups
/// and may be damaged.
///
/// S2-SVD servo: STEP/DIR sink the drive's opto inputs directly, no level
/// shifter: CN2 PP+ (3) and PD+ (4) to 3V3, PP- (14) to STEP, PD- (5) to DIR,
/// GND (1) to GND. ENA is unused (the drive's enable is a 12-24 V input): set
/// Pn003 = 1 so the drive enables itself at power-up.
///
/// Drive tuning on this machine: Fn018 inertia ratio ~1.5 (hands off the
/// carriage) in Pn257, automatic gains (Pn258), rigidity Pn259 = 8 (9 and up
/// hums), online inertia estimation off (Pn260 = 0), command smoothing off
/// (Pn109 = 0). Save with Fn001 and power-cycle.
#[cfg(all(feature = "esp32c6", not(feature = "breadboard")))]
macro_rules! take_pins {
    ($p:ident) => {
        $crate::config::Pins {
            encoder_a: esp_hal::gpio::Pin::degrade($p.GPIO10),
            encoder_b: esp_hal::gpio::Pin::degrade($p.GPIO11),
            step: esp_hal::gpio::Pin::degrade($p.GPIO18),
            dir: esp_hal::gpio::Pin::degrade($p.GPIO19),
            enable: esp_hal::gpio::Pin::degrade($p.GPIO20),
            nextion_tx: esp_hal::gpio::Pin::degrade($p.GPIO22),
            nextion_rx: esp_hal::gpio::Pin::degrade($p.GPIO23),
        }
    };
}

/// BREADBOARD BUILD (`--features breadboard`), for test-driving on a solder
/// breadboard. Not for a PCB.
///
/// A 74HCT125 (VCC 5 V) sits in the valley under the DevKitC-1 with pin 14
/// on the 5V row, so every IC pin shares a strip with a header pin:
///
/// | IC pin | strip    | use                                            |
/// |--------|----------|------------------------------------------------|
/// | 14 VCC | 5V       | supply, 100 nF to pin 7                        |
/// | 5  2A  | GPIO18   | STEP in                                        |
/// | 6  2Y  | GPIO19   | STEP out to CN2 14 (PP-); GPIO19 left unused   |
/// | 4  2OE | GPIO9    | ENA, active low (BOOT strap: high during reset,|
/// |        |          | so the outputs are off while the chip boots)   |
/// | 9  3A  | GPIO8    | DIR in                                         |
/// | 8  3Y  | GPIO1    | DIR out to CN2 5 (PD-); GPIO1 left unused      |
/// | 10 3OE | GPIO10   | jumpered to the GPIO9 strip; GPIO10 unused     |
/// | 7  GND | GPIO20   | jumpered to a G pin; GPIO20 unused (never      |
/// |        |          | driven: it's ground now)                       |
/// | 1, 13  | 12, 3    | bent out, tied to pin 14 (channels 1, 4 off)   |
/// | 2, 12  | 13, 2    | bent out, tied to GND                          |
/// | 3, 11  | G, 11    | cut off                                        |
///
/// CN2 PP+ (3) and PD+ (4) go to 5V. The encoder moves to GPIO21/15
/// (GPIO10/11 are taken, and GPIO0-7 are LP pads, see encoder-log), each
/// with its own 4.7k pull-up to 3V3. ENA is forced active-low (`invert_enable`).
#[cfg(all(feature = "esp32c6", feature = "breadboard"))]
macro_rules! take_pins {
    ($p:ident) => {
        $crate::config::Pins {
            encoder_a: esp_hal::gpio::Pin::degrade($p.GPIO21),
            encoder_b: esp_hal::gpio::Pin::degrade($p.GPIO15),
            step: esp_hal::gpio::Pin::degrade($p.GPIO18),
            dir: esp_hal::gpio::Pin::degrade($p.GPIO8),
            enable: esp_hal::gpio::Pin::degrade($p.GPIO9),
            nextion_tx: esp_hal::gpio::Pin::degrade($p.GPIO22),
            nextion_rx: esp_hal::gpio::Pin::degrade($p.GPIO23),
        }
    };
}

#[cfg(all(feature = "breadboard", not(feature = "esp32c6")))]
compile_error!("the breadboard pinout is for the ESP32-C6 DevKitC-1 only");

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
    // Lead screw pitch in deci-microns (20000 = 2mm, measured); motor steps
    // per screw turn (pulses per motor turn * gear ratio). The S2-SVD servo
    // takes 10000 pulses per turn with its default 1:1 electronic gear.
    screw_du: 20_000,
    motor_steps: 10_000,
    // steps/s, steps/s, steps/s². 100k steps/s = 600 rpm = 20 mm/s, under
    // the ~124k limit of 32 steps per 250 µs segment. The carriage follows
    // spindle speed changes only within `acceleration`: 2M steps/s² (0-600
    // rpm in 50 ms) keeps up with a 2 mm thread slowing 12,000 rpm/s.
    speed_start: 2_000,
    speed_max: 100_000,
    acceleration: 2_000_000,
    // Largest pitch accepted, deci-microns (254000 = 1").
    max_pitch_du: 254_000,
    // Driver signals. STEP idles high and pulses low on the NanoEls H5.
    invert_dir: false,
    // The breadboard build gates the buffer's active-low OE with ENA.
    invert_enable: cfg!(feature = "breadboard"),
    step_active_low: true,
    step_pulse_ns: 2_500,
    dir_setup_ns: 10_000,
    // Jog speeds in du/s (0.5, 2, 8, 25 mm/s), one per jog distance (.01 / .1 /
    // 1 / 10). Taps use the selected distance's speed; holds start there and
    // step up to the fastest.
    jog_speeds_du_per_s: [5_000, 20_000, 80_000, 250_000],
    jog_hold_after_ms: 400,
    jog_level_every_ms: 1_000,
    // Lead screw backlash in du, taken up on every reversal. Measure it with
    // the wizard on the setup page.
    backlash_du: 0,
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

/// The display's rate after power-up: `baud=` in the HMI's Program.s (the
/// Nextion Editor default is 9600)...
pub const NEXTION_BOOT_BAUD: u32 = 9_600;
/// ...and the rate the firmware switches it to at boot.
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
