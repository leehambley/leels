//! Machine settings editable from the setup web page, and operator state
//! remembered across power cycles. Both are persisted with [`crate::record`].

use crate::jog::{JogConfig, JOG_LEVELS};
use crate::pitch::{Pitch, Unit, JOG_SIZES_MILLI, STEP_SIZES_MILLI};
use crate::record::{self, Loaded, Reader, Writer};
use crate::stepgen::{StepGenConfig, MAX_STEPS_PER_SEGMENT};
use crate::Machine;

pub const SETTINGS_MAGIC: u32 = u32::from_le_bytes(*b"LELS");
pub const SETTINGS_VERSION: u16 = 1;
pub const STATE_MAGIC: u32 = u32::from_le_bytes(*b"LEOP");
pub const STATE_VERSION: u16 = 1;
/// Upper bound on an encoded record (header + payload) of either kind.
pub const MAX_RECORD_LEN: usize = 128;

/// Machine and motion settings. Pins are fixed in the firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub encoder_ppr: u32,
    pub encoder_backlash: u32,
    pub encoder_filter_cycles: u16,
    pub invert_spindle: bool,
    pub screw_du: u32,
    pub motor_steps: u32,
    pub speed_start: u32,
    pub speed_max: u32,
    pub acceleration: u32,
    pub max_pitch_du: u32,
    pub invert_dir: bool,
    pub invert_enable: bool,
    pub step_active_low: bool,
    pub step_pulse_ns: u32,
    pub dir_setup_ns: u32,
    pub jog_speeds_du_per_s: [u32; JOG_LEVELS],
    pub jog_hold_after_ms: u32,
    pub jog_level_every_ms: u32,
}

/// Fixed motion timing from the firmware, needed to validate settings.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub tick_hz: u32,
    pub segment_ticks: u32,
}

/// Where the settings in use came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Saved,
    /// Nothing saved yet: firmware defaults.
    FirstBoot,
    /// Saved data was damaged or failed validation: firmware defaults.
    Corrupt,
    /// Saved by a newer firmware with an unknown format: firmware defaults.
    Unsupported,
}

impl Settings {
    pub fn machine(&self) -> Machine {
        Machine {
            counts_per_rev: self.encoder_ppr as i64 * 2,
            motor_steps: self.motor_steps as i64,
            screw_du: self.screw_du as i64,
        }
    }

    pub fn stepgen(&self, t: Timing) -> StepGenConfig {
        let ticks = |ns: u32| (ns as u64 * t.tick_hz as u64).div_ceil(1_000_000_000) as u32;
        StepGenConfig {
            tick_hz: t.tick_hz,
            segment_ticks: t.segment_ticks,
            pulse_ticks: ticks(self.step_pulse_ns).max(1),
            dir_setup_ticks: ticks(self.dir_setup_ns),
            start_speed: self.speed_start,
            max_speed: self.speed_max,
            acceleration: self.acceleration,
        }
    }

    pub fn jog(&self) -> JogConfig {
        let m = self.machine();
        JogConfig {
            hold_after_us: self.jog_hold_after_ms as u64 * 1_000,
            level_every_us: self.jog_level_every_ms as u64 * 1_000,
            speeds: self.jog_speeds_du_per_s.map(|du| m.du_to_steps(du as i64).clamp(1, self.speed_max as i64) as u32),
        }
    }

    /// Check every field's range and the combinations the motion code relies on.
    pub fn validate(&self, t: Timing) -> Result<(), &'static str> {
        for f in FIELDS {
            let v = (f.get)(self);
            if v < f.min || v > f.max {
                return Err(f.label);
            }
        }
        if self.speed_start > self.speed_max {
            return Err("Start speed must not exceed max speed");
        }
        let g = self.stepgen(t);
        if self.speed_max > g.pulse_limited_speed() {
            return Err("STEP pulse too wide for max speed");
        }
        if g.max_steps_per_segment() > MAX_STEPS_PER_SEGMENT as u64 {
            return Err("Max speed too high for the control period");
        }
        if g.dir_setup_ticks >= g.segment_ticks / 2 {
            return Err("DIR setup time too long");
        }
        Ok(())
    }

    pub fn encode(&self, sequence: u32, out: &mut [u8]) -> Option<usize> {
        let mut payload = [0u8; MAX_RECORD_LEN - record::HEADER_LEN];
        let mut w = Writer::new(&mut payload);
        w.u32(self.encoder_ppr);
        w.u32(self.encoder_backlash);
        w.u16(self.encoder_filter_cycles);
        w.bool(self.invert_spindle);
        w.u32(self.screw_du);
        w.u32(self.motor_steps);
        w.u32(self.speed_start);
        w.u32(self.speed_max);
        w.u32(self.acceleration);
        w.u32(self.max_pitch_du);
        w.bool(self.invert_dir);
        w.bool(self.invert_enable);
        w.bool(self.step_active_low);
        w.u32(self.step_pulse_ns);
        w.u32(self.dir_setup_ns);
        for s in self.jog_speeds_du_per_s {
            w.u32(s);
        }
        w.u32(self.jog_hold_after_ms);
        w.u32(self.jog_level_every_ms);
        let len = w.finish()?;
        record::encode(SETTINGS_MAGIC, SETTINGS_VERSION, sequence, &payload[..len], out)
    }

    fn decode_v1(payload: &[u8]) -> Option<Self> {
        let mut r = Reader::new(payload);
        let s = Settings {
            encoder_ppr: r.u32()?,
            encoder_backlash: r.u32()?,
            encoder_filter_cycles: r.u16()?,
            invert_spindle: r.bool()?,
            screw_du: r.u32()?,
            motor_steps: r.u32()?,
            speed_start: r.u32()?,
            speed_max: r.u32()?,
            acceleration: r.u32()?,
            max_pitch_du: r.u32()?,
            invert_dir: r.bool()?,
            invert_enable: r.bool()?,
            step_active_low: r.bool()?,
            step_pulse_ns: r.u32()?,
            dir_setup_ns: r.u32()?,
            jog_speeds_du_per_s: [r.u32()?, r.u32()?, r.u32()?, r.u32()?],
            jog_hold_after_ms: r.u32()?,
            jog_level_every_ms: r.u32()?,
        };
        r.done().then_some(s)
    }

    /// Settings to run with, given what was found in flash.
    pub fn from_loaded(loaded: &Loaded, defaults: Settings, t: Timing) -> (Settings, Origin) {
        match loaded {
            Loaded::Blank => (defaults, Origin::FirstBoot),
            Loaded::Corrupt => (defaults, Origin::Corrupt),
            Loaded::Found { record, .. } => {
                let decoded = match record.version {
                    1 => Self::decode_v1(record.payload),
                    // Future versions: add migrations here, e.g. `2 => decode_v2(..)`.
                    _ => return (defaults, Origin::Unsupported),
                };
                match decoded {
                    Some(s) if s.validate(t).is_ok() => (s, Origin::Saved),
                    _ => (defaults, Origin::Corrupt),
                }
            }
        }
    }
}

/// How a setting is shown and entered on the web page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Integer,
    /// Stored in 1/10000ths (du for mm, du/s for mm/s), shown with 4 decimals.
    Decimal4,
    Flag,
}

/// One editable setting. `get`/`set` work on the stored integer.
pub struct Field {
    pub key: &'static str,
    pub section: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    pub min: i64,
    pub max: i64,
    pub get: fn(&Settings) -> i64,
    pub set: fn(&mut Settings, i64),
}

macro_rules! field {
    ($key:literal, $section:literal, $label:literal, $unit:literal, $help:literal, $kind:ident, $min:expr, $max:expr, $($f:tt)+) => {
        Field {
            key: $key,
            section: $section,
            label: $label,
            unit: $unit,
            help: $help,
            kind: Kind::$kind,
            min: $min,
            max: $max,
            get: |s| s.$($f)+ as i64,
            set: |s, v| s.$($f)+ = field!(@cast $kind, v, s.$($f)+),
        }
    };
    (@cast Flag, $v:ident, $t:expr) => { $v != 0 };
    (@cast $k:ident, $v:ident, $t:expr) => { $v.try_into().unwrap_or_default() };
}

const ENC: &str = "Spindle encoder";
const SCREW: &str = "Lead screw and motor";
const DRIVER: &str = "Driver signals";
const JOG: &str = "Jogging";

pub const FIELDS: &[Field] = &[
    field!(
        "encoder_ppr",
        "Spindle encoder",
        "Encoder lines per revolution",
        "lines",
        "From the encoder datasheet. Two counts are taken per line.",
        Integer,
        1,
        15_000,
        encoder_ppr
    ),
    field!(
        "encoder_backlash",
        "Spindle encoder",
        "Encoder backlash",
        "counts",
        "Reversal the carriage ignores, to filter encoder jitter.",
        Integer,
        0,
        1_000,
        encoder_backlash
    ),
    field!(
        "encoder_filter",
        "Spindle encoder",
        "Encoder glitch filter",
        "12.5 ns cycles",
        "Pulses shorter than this are ignored. 40 = 0.5 µs.",
        Integer,
        0,
        1_023,
        encoder_filter_cycles
    ),
    field!(
        "invert_spindle",
        "Spindle encoder",
        "Invert spindle direction",
        "",
        "Tick if a positive pitch drives the carriage the wrong way (or swap A/B).",
        Flag,
        0,
        1,
        invert_spindle
    ),
    field!(
        "screw_pitch",
        "Lead screw and motor",
        "Lead screw pitch",
        "mm",
        "Carriage travel per lead screw turn. Inch screw: 25.4 / TPI.",
        Decimal4,
        100,
        10_000_000,
        screw_du
    ),
    field!(
        "motor_steps",
        "Lead screw and motor",
        "Motor steps per screw turn",
        "steps",
        "Full steps × microsteps × gear ratio. 200 × 4 = 800.",
        Integer,
        1,
        1_000_000,
        motor_steps
    ),
    field!(
        "speed_start",
        "Lead screw and motor",
        "Start speed",
        "steps/s",
        "Speed the motor can start and stop at without ramping.",
        Integer,
        1,
        1_000_000,
        speed_start
    ),
    field!(
        "speed_max",
        "Lead screw and motor",
        "Max speed",
        "steps/s",
        "Highest step rate the motor and driver can follow.",
        Integer,
        1,
        1_000_000,
        speed_max
    ),
    field!(
        "acceleration",
        "Lead screw and motor",
        "Acceleration",
        "steps/s²",
        "Higher feels snappier; too high loses steps.",
        Integer,
        1,
        100_000_000,
        acceleration
    ),
    field!(
        "max_pitch",
        "Lead screw and motor",
        "Largest pitch",
        "mm",
        "Pitch entries above this are refused.",
        Decimal4,
        10,
        1_000_000,
        max_pitch_du
    ),
    field!(
        "invert_dir",
        "Driver signals",
        "Invert DIR",
        "",
        "Tick if the carriage moves opposite to the pitch sign.",
        Flag,
        0,
        1,
        invert_dir
    ),
    field!(
        "invert_enable",
        "Driver signals",
        "Invert ENABLE",
        "",
        "Tick if the driver's enable input is active-low.",
        Flag,
        0,
        1,
        invert_enable
    ),
    field!(
        "step_active_low",
        "Driver signals",
        "STEP pulses low",
        "",
        "Tick if STEP idles high and pulses low (NanoEls H5).",
        Flag,
        0,
        1,
        step_active_low
    ),
    field!(
        "step_pulse",
        "Driver signals",
        "STEP pulse width",
        "ns",
        "Driver datasheet minimum, with margin.",
        Integer,
        100,
        100_000,
        step_pulse_ns
    ),
    field!(
        "dir_setup",
        "Driver signals",
        "DIR setup time",
        "ns",
        "DIR must be stable this long before a STEP edge.",
        Integer,
        0,
        100_000,
        dir_setup_ns
    ),
    field!(
        "jog_speed_1",
        "Jogging",
        "Hold speed 1",
        "mm/s",
        "First speed while a jog button is held.",
        Decimal4,
        1,
        10_000_000,
        jog_speeds_du_per_s[0]
    ),
    field!("jog_speed_2", "Jogging", "Hold speed 2", "mm/s", "", Decimal4, 1, 10_000_000, jog_speeds_du_per_s[1]),
    field!("jog_speed_3", "Jogging", "Hold speed 3", "mm/s", "", Decimal4, 1, 10_000_000, jog_speeds_du_per_s[2]),
    field!(
        "jog_speed_4",
        "Jogging",
        "Hold speed 4",
        "mm/s",
        "Also the speed of a single tap move.",
        Decimal4,
        1,
        10_000_000,
        jog_speeds_du_per_s[3]
    ),
    field!(
        "jog_hold_after",
        "Jogging",
        "Hold after",
        "ms",
        "Press longer than this to run continuously.",
        Integer,
        50,
        5_000,
        jog_hold_after_ms
    ),
    field!(
        "jog_level_every",
        "Jogging",
        "Speed up every",
        "ms",
        "Time at each hold speed before the next.",
        Integer,
        100,
        60_000,
        jog_level_every_ms
    ),
];

/// Section headings in page order.
pub const SECTIONS: [&str; 4] = [ENC, SCREW, DRIVER, JOG];

/// Operator choices remembered across power cycles. Stops and position are
/// deliberately not saved: the carriage may be moved while powered off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorState {
    pub pitch: Pitch,
    pub step_idx: u8,
    pub jog_idx: u8,
}

impl OperatorState {
    pub fn encode(&self, sequence: u32, out: &mut [u8]) -> Option<usize> {
        let mut payload = [0u8; 16];
        let mut w = Writer::new(&mut payload);
        w.u8(match self.pitch.unit {
            Unit::Mm => 0,
            Unit::Inch => 1,
        });
        w.u32(self.pitch.magnitude_milli);
        w.bool(self.pitch.reversed);
        w.u8(self.step_idx);
        w.u8(self.jog_idx);
        let len = w.finish()?;
        record::encode(STATE_MAGIC, STATE_VERSION, sequence, &payload[..len], out)
    }

    /// Decode a saved state; `None` if absent, damaged or out of range.
    pub fn from_loaded(loaded: &Loaded, max_pitch_du: i64) -> Option<Self> {
        let Loaded::Found { record, .. } = loaded else { return None };
        if record.version != 1 {
            return None;
        }
        let mut r = Reader::new(record.payload);
        let unit = match r.u8()? {
            0 => Unit::Mm,
            1 => Unit::Inch,
            _ => return None,
        };
        let pitch = Pitch { unit, magnitude_milli: r.u32()?, reversed: r.bool()? };
        let s = OperatorState { pitch, step_idx: r.u8()?, jog_idx: r.u8()? };
        let ok = r.done()
            && pitch.du().abs() <= max_pitch_du
            && (s.step_idx as usize) < STEP_SIZES_MILLI.len()
            && (s.jog_idx as usize) < JOG_SIZES_MILLI.len();
        ok.then_some(s)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::record::{load, SlotError};

    pub const TIMING: Timing = Timing { tick_hz: 10_000_000, segment_ticks: 2_500 };

    pub const DEFAULTS: Settings = Settings {
        encoder_ppr: 1200,
        encoder_backlash: 3,
        encoder_filter_cycles: 40,
        invert_spindle: false,
        screw_du: 40_000,
        motor_steps: 800,
        speed_start: 800,
        speed_max: 30_000,
        acceleration: 20_000,
        max_pitch_du: 254_000,
        invert_dir: false,
        invert_enable: false,
        step_active_low: true,
        step_pulse_ns: 2_500,
        dir_setup_ns: 10_000,
        jog_speeds_du_per_s: [5_000, 20_000, 80_000, 250_000],
        jog_hold_after_ms: 400,
        jog_level_every_ms: 1_000,
    };

    fn saved(s: &Settings, seq: u32) -> [u8; MAX_RECORD_LEN] {
        let mut buf = [0xFF; MAX_RECORD_LEN];
        s.encode(seq, &mut buf).unwrap();
        buf
    }

    #[test]
    fn defaults_are_valid() {
        assert_eq!(DEFAULTS.validate(TIMING), Ok(()));
    }

    #[test]
    fn round_trips_through_flash_record() {
        let mut s = DEFAULTS;
        s.screw_du = 50_800;
        s.invert_dir = true;
        s.jog_speeds_du_per_s[2] = 123_456;
        let a = saved(&s, 3);
        let blank = [0xFF; MAX_RECORD_LEN];
        let (got, origin) = Settings::from_loaded(&load(SETTINGS_MAGIC, [&a, &blank]), DEFAULTS, TIMING);
        assert_eq!((got, origin), (s, Origin::Saved));
    }

    #[test]
    fn first_boot_corrupt_and_unsupported_fall_back_to_defaults() {
        let blank = [0xFF; MAX_RECORD_LEN];
        let loaded = load(SETTINGS_MAGIC, [&blank, &blank]);
        assert_eq!(Settings::from_loaded(&loaded, DEFAULTS, TIMING).1, Origin::FirstBoot);

        let noise = [0x5A; MAX_RECORD_LEN];
        let loaded = load(SETTINGS_MAGIC, [&noise, &blank]);
        assert_eq!(Settings::from_loaded(&loaded, DEFAULTS, TIMING).1, Origin::Corrupt);

        let mut future = [0xFF; MAX_RECORD_LEN];
        record::encode(SETTINGS_MAGIC, 99, 1, b"whatever", &mut future).unwrap();
        let loaded = load(SETTINGS_MAGIC, [&future, &blank]);
        assert_eq!(Settings::from_loaded(&loaded, DEFAULTS, TIMING).1, Origin::Unsupported);
    }

    #[test]
    fn valid_record_with_invalid_values_is_rejected() {
        let mut s = DEFAULTS;
        s.motor_steps = 0;
        let a = saved(&s, 1);
        let blank = [0xFF; MAX_RECORD_LEN];
        let (got, origin) = Settings::from_loaded(&load(SETTINGS_MAGIC, [&a, &blank]), DEFAULTS, TIMING);
        assert_eq!((got, origin), (DEFAULTS, Origin::Corrupt));
    }

    #[test]
    fn settings_record_fits() {
        let buf = saved(&DEFAULTS, 1);
        assert!(record::decode(SETTINGS_MAGIC, &buf).is_ok());
        assert!(matches!(record::decode(STATE_MAGIC, &buf), Err(SlotError::Corrupt)));
    }

    #[test]
    fn cross_field_validation() {
        let mut s = DEFAULTS;
        s.speed_start = 40_000;
        assert!(s.validate(TIMING).is_err());
        let mut s = DEFAULTS;
        s.speed_max = 300_000; // above what 2.5 µs pulses allow
        assert!(s.validate(TIMING).is_err());
        let mut s = DEFAULTS;
        s.step_pulse_ns = 50_000; // 50 µs pulses can't reach 30k steps/s
        assert!(s.validate(TIMING).is_err());
    }

    #[test]
    fn fields_get_and_set() {
        let mut s = DEFAULTS;
        let f = FIELDS.iter().find(|f| f.key == "jog_speed_3").unwrap();
        (f.set)(&mut s, 42);
        assert_eq!(s.jog_speeds_du_per_s[2], 42);
        let f = FIELDS.iter().find(|f| f.key == "invert_enable").unwrap();
        (f.set)(&mut s, 1);
        assert!(s.invert_enable);
        assert_eq!((f.get)(&s), 1);
        assert!(FIELDS.iter().all(|f| SECTIONS.contains(&f.section)));
    }

    #[test]
    fn operator_state_round_trip_and_bounds() {
        let st = OperatorState {
            pitch: Pitch { magnitude_milli: 1_250, reversed: true, unit: Unit::Inch },
            step_idx: 3,
            jog_idx: 2,
        };
        let mut a = [0xFF; MAX_RECORD_LEN];
        st.encode(9, &mut a).unwrap();
        let blank = [0xFF; MAX_RECORD_LEN];
        let loaded = load(STATE_MAGIC, [&blank, &a]);
        // 1.25" exceeds a 1" limit.
        assert_eq!(OperatorState::from_loaded(&loaded, 254_000), None);
        assert_eq!(OperatorState::from_loaded(&loaded, 1_000_000), Some(st));
        assert_eq!(OperatorState::from_loaded(&load(STATE_MAGIC, [&blank, &blank]), 254_000), None);
    }
}
