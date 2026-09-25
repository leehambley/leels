//! Renders UI + motion status into the text of each Nextion component.

use core::fmt::Write;

use crate::gearbox::Side;
use crate::jog::JogView;
use crate::pitch::Unit;
use crate::ui::Ui;
use crate::{mul_div_round, Machine};

/// Snapshot published by the motion task.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub engaged: bool,
    pub syncing: bool,
    /// A jog move is in progress.
    pub jogging: bool,
    pub jog: JogView,
    /// Motor position in steps.
    pub pos: i64,
    /// Motor position shown as Z = 0.
    pub z_zero: i64,
    pub left_stop: Option<i64>,
    pub right_stop: Option<i64>,
    pub rpm: u32,
    /// Spindle counts since turns/angle were last zeroed.
    pub turn_counts: i64,
}

/// Longest field is `tMessageLine` (40 characters in the HMI).
pub type Text = heapless::String<40>;

/// What a Z stop button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopState {
    Unset,
    /// Set, carriage elsewhere.
    Set,
    /// Carriage is sitting at the stop (threading pass ended, or jogged into it).
    Parked,
}

impl Status {
    pub fn stop_state(&self, side: Side) -> StopState {
        let stop = match side {
            Side::Left => self.left_stop,
            Side::Right => self.right_stop,
        };
        match stop {
            None => StopState::Unset,
            Some(s) if s == self.pos => StopState::Parked,
            Some(_) => StopState::Set,
        }
    }
}

/// Button `(background, text)` colours (RGB565) for the stop states, indexed
/// by [`StopState`]. Every pair has at least 7:1 WCAG contrast (AAA) for
/// poor workshop lighting; see the `stop_colours_meet_wcag_aaa` test.
/// `Unset` is the HMI's normal grey button with black text.
pub const STOP_COLOURS: [(u16, u16); 3] = [
    (0xC618, 0x0000), // grey / black, 11.9:1
    (0xFD80, 0x0000), // amber / black, 11.6:1
    (0xA000, 0xFFFF), // dark red / white, 8.1:1
];

/// `tPitch`: normal (white on the page's dark grey, 14.1:1), and while a
/// keypad entry is pending (black on amber, 11.6:1).
pub const PITCH_COLOURS: [(u16, u16); 2] = [(0x2965, 0xFFFF), (0xFD80, 0x0000)];

/// `bReverseToggle`: normal direction (grey), reversed (amber).
pub const REVERSE_COLOURS: [(u16, u16); 2] = [(0xC618, 0x0000), (0xFD80, 0x0000)];

/// Components whose `(bco, pco)` the firmware sets, in [`field_colours`] order.
pub const COLOUR_FIELDS: [&str; 4] = ["bLeftStop", "bRightStop", "tPitch", "bReverseToggle"];

/// `(bco, pco)` of each of [`COLOUR_FIELDS`].
pub fn field_colours(ui: &Ui, st: &Status) -> [(u16, u16); COLOUR_FIELDS.len()] {
    [
        STOP_COLOURS[st.stop_state(Side::Left) as usize],
        STOP_COLOURS[st.stop_state(Side::Right) as usize],
        PITCH_COLOURS[ui.entry().is_some() as usize],
        REVERSE_COLOURS[ui.pitch.reversed as usize],
    ]
}

/// Keypad cursor blink half-period.
const CURSOR_BLINK_MS: u64 = 500;

/// Nextion object names (Lee-LS HMI), in the order `render` returns their
/// text. Several are buttons whose caption carries the current value.
pub const FIELDS: [&str; 12] = [
    "bToggleEngaged",
    "tPitch",
    "bUnitsToggle",
    "tRPM",
    "tTurns",
    "tAngle",
    "tZPos",
    "bLeftStop",
    "bRightStop",
    "bCycleJogDist",
    "tMessageLine",
    "bReverseToggle",
];

/// `setup_banner` is shown on the message line while setup mode is active.
pub fn render(ui: &Ui, st: &Status, m: &Machine, now_ms: u64, setup_banner: &str) -> [Text; FIELDS.len()] {
    let unit = ui.pitch.unit;
    // The engage button's caption says what it shows/does now.
    let status = if ui.setup_active() {
        "Setup on"
    } else if st.syncing {
        "Syncing"
    } else if st.engaged {
        "Disengage"
    } else {
        "Engage"
    };
    let n = m.counts_per_rev;
    let turns = fixed(st.turn_counts.abs() * 100 / n, 2);
    let mut angle = fixed(st.turn_counts.rem_euclid(n) * 36_000 / n, 2);
    let _ = angle.push('°');

    // A pending keypad entry replaces the pitch readout, with a blinking
    // cursor (and amber colours, see `field_colours`) until OK.
    let pitch = match ui.entry() {
        Some(e) => {
            let cursor = if (now_ms / CURSOR_BLINK_MS).is_multiple_of(2) { "_" } else { "" };
            labelled("PITCH ", e, cursor)
        }
        None => labelled("PITCH ", &fixed(ui.pitch.signed_milli(), 3), unit.suffix()),
    };

    let message = if let Some(msg) = ui.message(now_ms) {
        text(msg)
    } else if ui.entry().is_some() {
        text("OK to apply, <- delete, hold <- clear")
    } else if ui.setup_active() {
        text(setup_banner)
    } else if st.syncing {
        text("Waiting for thread phase")
    } else if ui.pitch.magnitude_milli == 0 {
        text("Set pitch")
    } else {
        Text::new()
    };

    let mut rpm = Text::new();
    let _ = write!(rpm, "RPM {}", st.rpm);
    [
        text(status),
        pitch,
        // bUnitsToggle caption, max 10 characters in the HMI.
        labelled("UNIT: ", unit.label(), ""),
        rpm,
        labelled("TURNS ", &turns, ""),
        labelled("ANGLE ", &angle, ""),
        labelled("Z POS ", &distance(m, unit, st.pos - st.z_zero), unit.suffix()),
        stop_caption(st, m, unit, Side::Left),
        stop_caption(st, m, unit, Side::Right),
        jog_caption(ui, st, m, unit),
        message,
        text(if ui.pitch.reversed { "REVERSE" } else { "NORMAL" }),
    ]
}

fn text(s: &str) -> Text {
    let mut t = Text::new();
    for c in s.chars() {
        if t.push(c).is_err() {
            break;
        }
    }
    t
}

/// `bCycleJogDist`: the selected tap distance when idle, live jog feedback
/// while moving. At most 15 characters.
fn jog_caption(ui: &Ui, st: &Status, m: &Machine, unit: Unit) -> Text {
    let mut t = Text::new();
    match st.jog {
        JogView::Idle => return labelled("JOG DIST ", &trim_zeros(fixed(ui.jog_milli() as i64, 3)), unit.suffix()),
        JogView::Tap { remaining } => return labelled("JOG ", &distance(m, unit, remaining), unit.suffix()),
        JogView::Hold { level } => {
            let _ = write!(t, "HOLD SPEED {level}/{}", crate::jog::JOG_LEVELS);
        }
        JogView::Braking => t = text("STOPPING"),
    }
    t
}

/// At most 15 characters, e.g. `|< -123.456mm`.
fn stop_caption(st: &Status, m: &Machine, unit: Unit, side: Side) -> Text {
    let (stop, left) = match side {
        Side::Left => (st.left_stop, true),
        Side::Right => (st.right_stop, false),
    };
    let body = match (st.stop_state(side), stop) {
        (StopState::Parked, _) => text("AT STOP"),
        (StopState::Set, Some(s)) => labelled("", &distance(m, unit, s - st.z_zero), unit.suffix()),
        _ => text("Set stop"),
    };
    if left {
        labelled("|< ", &body, "")
    } else {
        labelled("", &body, " >|")
    }
}

fn labelled(prefix: &str, value: &str, suffix: &str) -> Text {
    let mut t = text(prefix);
    let _ = t.push_str(value);
    let _ = t.push_str(suffix);
    t
}

/// Distance in steps as mm (3 decimals) or inch (4 decimals).
fn distance(m: &Machine, unit: Unit, steps: i64) -> Text {
    let du = m.steps_to_du(steps);
    match unit {
        Unit::Mm => fixed(mul_div_round(du, 1, 10), 3),
        Unit::Inch => fixed(mul_div_round(du, 10, 254), 4),
    }
}

/// `value / 10^decimals` with exactly `decimals` digits after the point.
pub fn fixed(value: i64, decimals: u32) -> Text {
    let mut t = Text::new();
    let scale = 10i64.pow(decimals);
    let sign = if value < 0 { "-" } else { "" };
    let v = value.unsigned_abs();
    let s = scale as u64;
    let _ = if decimals == 0 {
        write!(t, "{sign}{v}")
    } else {
        write!(t, "{sign}{}.{:0w$}", v / s, v % s, w = decimals as usize)
    };
    t
}

fn trim_zeros(mut t: Text) -> Text {
    if t.contains('.') {
        while t.ends_with('0') {
            t.pop();
        }
        if t.ends_with('.') {
            t.pop();
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Key;

    const IDLE: Status = Status {
        engaged: false,
        syncing: false,
        jogging: false,
        jog: JogView::Idle,
        pos: 0,
        z_zero: 0,
        left_stop: None,
        right_stop: None,
        rpm: 0,
        turn_counts: 0,
    };

    const M: Machine = Machine { counts_per_rev: 2400, motor_steps: 800, screw_du: 40_000 };

    #[test]
    fn fixed_formats() {
        assert_eq!(fixed(1250, 3), "1.250");
        assert_eq!(fixed(-5, 3), "-0.005");
        assert_eq!(fixed(0, 2), "0.00");
        assert_eq!(fixed(7, 0), "7");
        assert_eq!(trim_zeros(fixed(10_000, 3)), "10");
        assert_eq!(trim_zeros(fixed(10, 3)), "0.01");
    }

    #[test]
    fn renders_all_fields() {
        let mut ui = Ui::new(254_000);
        ui.handle(Key::StepSize(3), 0, &IDLE);
        ui.handle(Key::Plus, 0, &IDLE);
        ui.handle(Key::Reverse, 0, &IDLE);
        let st = Status {
            engaged: true,
            pos: 250,
            z_zero: 50,
            left_stop: Some(450),
            right_stop: None,
            rpm: 300,
            turn_counts: 3600,
            ..IDLE
        };
        let f = render(&ui, &st, &M, 0, "");
        let f: Vec<&str> = f.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            f,
            [
                "Disengage",
                "PITCH -1.000mm",
                "UNIT: MM",
                "RPM 300",
                "TURNS 1.50",
                "ANGLE 180.00°",
                "Z POS 1.000mm",
                "|< 2.000mm",
                "Set stop >|",
                "JOG DIST 0.1mm",
                "",
                "REVERSE",
            ]
        );
    }

    #[test]
    fn stop_states_captions_and_colours() {
        let ui = Ui::new(254_000);
        let st = Status { pos: 250, z_zero: 50, left_stop: Some(250), right_stop: Some(-1_000_000), ..IDLE };
        assert_eq!(st.stop_state(Side::Left), StopState::Parked);
        assert_eq!(st.stop_state(Side::Right), StopState::Set);
        let f = render(&ui, &st, &M, 0, "");
        assert_eq!(f[7], "|< AT STOP");
        assert_eq!(f[8], "-5000.250mm >|");
        assert!(f[8].chars().count() <= 15);
        assert_eq!(field_colours(&ui, &st)[..2], [STOP_COLOURS[2], STOP_COLOURS[1]]);
        assert_eq!(field_colours(&ui, &IDLE)[..2], [STOP_COLOURS[0]; 2]);
        assert_eq!(render(&ui, &IDLE, &M, 0, "")[7], "|< Set stop");
    }

    /// WCAG 2 contrast ratio of two RGB565 colours.
    fn contrast(a: u16, b: u16) -> f64 {
        fn luminance(c: u16) -> f64 {
            let ch = |v: u16, max: f64| {
                let v = v as f64 / max;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c >> 11, 31.0) + 0.7152 * ch((c >> 5) & 63, 63.0) + 0.0722 * ch(c & 31, 31.0)
        }
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn colours_meet_wcag_aaa() {
        for (bco, pco) in STOP_COLOURS.into_iter().chain(PITCH_COLOURS).chain(REVERSE_COLOURS) {
            let r = contrast(bco, pco);
            assert!(r >= 7.0, "{bco:#06x} on {pco:#06x} is only {r:.2}:1");
        }
        // The old bright red with black text would fail.
        assert!(contrast(0xF800, 0x0000) < 7.0);
    }

    #[test]
    fn jog_caption_follows_jog_state() {
        let ui = Ui::new(254_000);
        let cap = |jog| render(&ui, &Status { jog, ..IDLE }, &M, 0, "")[9].clone();
        assert_eq!(cap(JogView::Idle), "JOG DIST 0.1mm");
        assert_eq!(cap(JogView::Tap { remaining: 60 }), "JOG 0.300mm");
        assert_eq!(cap(JogView::Hold { level: 2 }), "HOLD SPEED 2/4");
        assert_eq!(cap(JogView::Braking), "STOPPING");
        for jog in [JogView::Tap { remaining: 200_000 }, JogView::Hold { level: 4 }] {
            assert!(cap(jog).chars().count() <= 15, "{}", cap(jog));
        }
    }

    #[test]
    fn renders_inch_and_entry() {
        let mut ui = Ui::new(254_000);
        ui.handle(Key::ToggleUnit, 0, &IDLE);
        ui.handle(Key::Digit(4), 0, &IDLE);
        let st = Status { pos: 254, ..IDLE }; // 254 steps = 1.27mm = 0.05"
        let f = render(&ui, &st, &M, 0, "");
        assert_eq!(f[2], "UNIT: IN");
        assert_eq!(f[6], "Z POS 0.0500in");
        assert_eq!(f[9], "JOG DIST 0.1in");
        assert_eq!(f[1], "PITCH 4_");
        assert_eq!(render(&ui, &st, &M, CURSOR_BLINK_MS, "")[1], "PITCH 4");
        assert_eq!(f[10], "OK to apply, <- delete, hold <- clear");
        assert_eq!(field_colours(&ui, &st)[2], PITCH_COLOURS[1]);
        assert!(f[10].chars().count() <= 40);
    }
}
