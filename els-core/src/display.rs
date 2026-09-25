//! Renders UI + motion status into the text of each Nextion component.

use core::fmt::Write;

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

pub type Text = heapless::String<24>;

/// Nextion object names, in the order `render` returns their text.
pub const FIELDS: [&str; 12] = [
    "bStatus", "tPitch", "bMeasure", "tStepVal", "tRPMVal", "tTurnsVal", "tAngleVal", "tZ", "tZLeft", "tZRight", "t3",
    "tJogVal",
];

pub fn render(ui: &Ui, st: &Status, m: &Machine, now_ms: u64) -> [Text; FIELDS.len()] {
    let unit = ui.pitch.unit;
    let status = if st.syncing {
        "SYN"
    } else if st.engaged {
        "ON"
    } else {
        "OFF"
    };
    let n = m.counts_per_rev;
    let turns = fixed(st.turn_counts.abs() * 100 / n, 2);
    let mut angle = fixed(st.turn_counts.rem_euclid(n) * 36_000 / n, 2);
    let _ = angle.push('°');

    let message = if let Some(e) = ui.entry() {
        let mut t = text("Pitch ");
        let _ = t.push_str(e);
        t
    } else if let Some(msg) = ui.message(now_ms) {
        text(msg)
    } else if st.syncing {
        text("Waiting for thread phase")
    } else if ui.pitch.magnitude_milli == 0 {
        text("Set pitch")
    } else {
        Text::new()
    };

    [
        text(status),
        fixed(ui.pitch.signed_milli(), 3),
        text(unit.label()),
        trim_zeros(fixed(ui.step_milli() as i64, 3)),
        {
            let mut t = Text::new();
            let _ = write!(t, "{}", st.rpm);
            t
        },
        turns,
        angle,
        distance(m, unit, st.pos - st.z_zero),
        st.left_stop.map_or_else(Text::new, |l| distance(m, unit, l - st.pos)),
        st.right_stop.map_or_else(Text::new, |r| distance(m, unit, st.pos - r)),
        message,
        trim_zeros(fixed(ui.jog_milli() as i64, 3)),
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
        let f = render(&ui, &st, &M, 0);
        let f: Vec<&str> = f.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            f,
            ["ON", "-1.000", "MM", "1", "300", "1.50", "180.00°", "1.000", "1.000", "", "", "0.1"]
        );
    }

    #[test]
    fn renders_inch_and_entry() {
        let mut ui = Ui::new(254_000);
        ui.handle(Key::ToggleUnit, 0, &IDLE);
        ui.handle(Key::Digit(4), 0, &IDLE);
        let st = Status { pos: 254, ..IDLE }; // 254 steps = 1.27mm = 0.05"
        let f = render(&ui, &st, &M, 0);
        assert_eq!(f[2], "IN");
        assert_eq!(f[7], "0.0500");
        assert_eq!(f[10], "Pitch 4");
    }
}
