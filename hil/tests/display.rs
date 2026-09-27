//! Operator-level tests on real hardware: touch the screen, check what the
//! display is told to show and what reaches the STEP pin.
//!
//! Machine defaults (config::DEFAULTS): 800 steps per 4 mm screw turn, so
//! 200 steps/mm. Jog right is towards negative Z.

use std::time::Duration;

use els_hil::{Board, AMBER, DARK_RED, GREY, LOCKED_BG, PITCH_NORMAL_BG};

macro_rules! board {
    () => {
        match Board::open() {
            Some(b) => b,
            None => return,
        }
    };
}

#[test]
fn boots_to_defaults() {
    let b = board!();
    assert_eq!(b.field("bToggleEngaged"), "Engage");
    assert_eq!(b.field("tPitch"), "↻0.000mm");
    assert_eq!(b.field("bUnitsToggle"), "UNIT: MM");
    assert_eq!(b.field("tZPos"), "⇄0.000");
    assert_eq!(b.field("bCycleJogDist"), "JOG DIST 0.1mm");
    assert_eq!(b.field("tMessageLine"), "Set pitch");
    assert_eq!(b.field("bReverseToggle"), "NORMAL");
    assert_eq!(b.field("bLeftStop"), "|< Set stop");
    assert_eq!(b.field("bRightStop"), "Set stop >|");
    assert_eq!(b.field("tAngle"), "ANGLE 0.00°");
}

#[test]
fn keypad_entry_is_amber_until_ok() {
    let mut b = board!();
    b.tap_all(&["bNum1", "bNumPeriod", "bNum2", "bNum5"]);
    b.expect_prefix("tPitch", "↻1.25");
    b.expect("tPitch.bco", AMBER);
    b.expect("tMessageLine", "OK to apply, <- delete, hold <- clear");
    b.tap("bNumOK");
    b.expect("tPitch", "↻1.250mm");
    b.expect("tPitch.bco", PITCH_NORMAL_BG);
    b.expect("tMessageLine", "");
}

#[test]
fn backspace_deletes_and_long_hold_clears() {
    let mut b = board!();
    b.tap_all(&["bNum1", "bNum2", "bNum3"]);
    b.expect_prefix("tPitch", "↻123");
    b.tap("bBackspace");
    b.expect_prefix("tPitch", "↻12");
    assert!(!b.field("tPitch").starts_with("↻123"));
    b.hold("bBackspace", Duration::from_millis(900));
    b.expect("tPitch", "↻0.000mm");
    b.expect("tPitch.bco", PITCH_NORMAL_BG);
}

#[test]
fn presets_units_and_reverse() {
    let mut b = board!();
    b.tap("bPitch05");
    b.expect("tPitch", "↻0.500mm");
    b.tap("bPitch005");
    b.expect("tPitch", "↻0.050mm");
    b.tap("bReverseToggle");
    b.expect("tPitch", "↻-0.050mm");
    b.expect("bReverseToggle", "REVERSE");
    b.expect("bReverseToggle.bco", AMBER);
    b.tap("bUnitsToggle");
    b.expect("tPitch", "↻-0.050in");
    b.expect("bUnitsToggle", "UNIT: IN");
    // 5.0" is over the 1" limit.
    b.tap("bPitch5");
    b.expect("tMessageLine", "Pitch too large");
    assert_eq!(b.field("tPitch"), "↻-0.050in");
}

#[test]
fn jog_tap_moves_exactly_one_distance() {
    let mut b = board!();
    b.tap("bJogR");
    b.expect("tZPos", "⇄-0.100");
    assert_eq!(b.settled_steps(), -20);
    b.tap("bJogL");
    b.expect("tZPos", "⇄0.000");
    assert_eq!(b.settled_steps(), 0);
    b.tap("bCycleJogDist");
    b.expect("bCycleJogDist", "JOG DIST 1mm");
    b.tap_all(&["bJogL", "bJogL"]);
    b.expect("tZPos", "⇄2.000");
    assert_eq!(b.settled_steps(), 400);
}

#[test]
fn jog_hold_runs_continuously_and_brakes_on_release() {
    let mut b = board!();
    b.press("bJogR");
    b.expect("bCycleJogDist", "HOLD SPEED 2/4");
    b.pump(Duration::from_millis(1_500));
    b.release("bJogR");
    b.expect("bCycleJogDist", "JOG DIST 0.1mm");
    let pos = b.settled_steps();
    // 0.4 s tap then ~1.5 s at 2 mm/s rising to 8 mm/s: well over 2 mm.
    assert!(pos < -400, "hold moved only {pos} steps");
}

#[test]
fn stops_limit_jogging_and_zero_z_clears_them() {
    let mut b = board!();
    b.tap("bJogR");
    b.expect("tZPos", "⇄-0.100");
    b.settled_steps();
    b.tap("bRightStop");
    b.expect("bRightStop", "AT STOP >|");
    b.expect("bRightStop.bco", DARK_RED);
    b.tap("bJogL");
    b.expect("tZPos", "⇄0.000");
    // Distance still to go to the stop.
    b.expect("bRightStop", "0.100mm >|");
    b.expect("bRightStop.bco", AMBER);
    // Two taps towards the stop: the second is cut short by it.
    b.tap_all(&["bJogR", "bJogR"]);
    b.expect("bRightStop", "AT STOP >|");
    assert_eq!(b.settled_steps(), -20);
    b.tap("bZeroZ");
    b.expect("tMessageLine", "Stops cleared");
    b.expect("tZPos", "⇄0.000");
    b.expect("bRightStop", "Set stop >|");
    b.expect("bRightStop.bco", GREY);
}

#[test]
fn engage_rules() {
    let mut b = board!();
    // Not with a half-typed pitch.
    b.tap("bNum1");
    b.tap("bToggleEngaged");
    b.expect("tMessageLine", "Press OK first");
    b.tap("bNumOK");
    b.expect("tPitch", "↻1.000mm");
    b.tap("bToggleEngaged");
    b.expect("bToggleEngaged", "Disengage");
    // While engaged: no jogging, no pitch changes.
    b.tap("bJogR");
    b.expect("tMessageLine", "Disengage to jog");
    b.tap("bPitch05");
    b.expect("tMessageLine", "Disengage to change pitch");
    b.expect("tPitch", "↻1.000mm");
    // The spindle isn't turning, so the carriage stays put.
    assert_eq!(b.settled_steps(), 0);
    b.tap("bToggleEngaged");
    b.expect("bToggleEngaged", "Engage");
}

#[test]
fn setup_mode_locks_everything_but_setup() {
    let mut b = board!();
    b.tap("bPitch05");
    b.expect("tPitch", "↻0.500mm");
    b.tap("bSettings");
    b.expect("bToggleEngaged", "Setup on");
    b.expect("tMessageLine", "WiFi LEELS-SETUP pw els-setup");
    for name in ["bJogR", "bNum1", "bPitch1", "bToggleEngaged", "bLeftStop", "tAngle"] {
        b.expect(&format!("{name}.tsw"), "0");
    }
    b.expect("bJogR.bco", LOCKED_BG);
    b.expect("bLeftStop.bco", LOCKED_BG);
    assert_eq!(b.field("bSettings.tsw"), "");
    // Touches that still reach the firmware are ignored.
    b.tap_all(&["bJogR", "bPitch1", "bNum1", "bToggleEngaged"]);
    b.pump(Duration::from_millis(500));
    assert_eq!(b.field("tPitch"), "↻0.500mm");
    assert_eq!(b.field("bToggleEngaged"), "Setup on");
    assert_eq!(b.settled_steps(), 0);
}
