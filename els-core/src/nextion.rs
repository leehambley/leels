//! Nextion serial protocol: touch event parsing, component → key mapping and
//! text command encoding.

use crate::ui::Key;

const TERM: [u8; 3] = [0xFF, 0xFF, 0xFF];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub page: u8,
    pub component: u8,
    pub pressed: bool,
}

/// Accumulates bytes from the display until a `FF FF FF` terminator.
#[derive(Default)]
pub struct Parser {
    buf: [u8; 32],
    len: usize,
}

impl Parser {
    pub fn push(&mut self, byte: u8) -> Option<Touch> {
        if self.len == self.buf.len() {
            self.len = 0;
        }
        self.buf[self.len] = byte;
        self.len += 1;
        if self.len < 3 || self.buf[self.len - 3..self.len] != TERM {
            return None;
        }
        let msg = &self.buf[..self.len - 3];
        self.len = 0;
        // Touch event: 0x65 page component event(1 = press, 0 = release).
        match *msg {
            [0x65, page, component, event] => Some(Touch { page, component, pressed: event == 1 }),
            _ => None,
        }
    }
}

/// Component ids on page 0 of `nextion/lee-ls.HMI` (checked against the
/// compiled `nextion/lee-ls.tft`). The Nextion Editor renumbers everything
/// after a deleted component, so re-check these whenever the HMI changes.
/// See docs/nextion.md.
pub mod id {
    pub const T_RPM: u8 = 1;
    pub const T_ANGLE: u8 = 2;
    pub const T_TURNS: u8 = 3;
    pub const B_LEFT_STOP: u8 = 4;
    pub const B_RIGHT_STOP: u8 = 5;
    pub const B_NUM1: u8 = 6;
    pub const B_NUM2: u8 = 7;
    pub const B_NUM3: u8 = 8;
    pub const B_BACKSPACE: u8 = 9;
    pub const B_NUM4: u8 = 10;
    pub const B_NUM5: u8 = 11;
    pub const B_NUM6: u8 = 12;
    pub const B_NUM7: u8 = 13;
    pub const B_NUM8: u8 = 14;
    pub const B_NUM9: u8 = 15;
    pub const B_NUM0: u8 = 16;
    pub const B_NUM_OK: u8 = 17;
    pub const B_NUM_PERIOD: u8 = 18;
    pub const B_TOGGLE_ENGAGED: u8 = 19;
    pub const B_REVERSE_TOGGLE: u8 = 20;
    pub const B_UNITS_TOGGLE: u8 = 21;
    pub const T_PITCH: u8 = 22;
    pub const B_PITCH_001: u8 = 23;
    pub const B_PITCH_01: u8 = 24;
    pub const B_PITCH_1: u8 = 25;
    pub const B_ZERO_Z: u8 = 26;
    pub const B_JOG_L: u8 = 27;
    pub const B_JOG_R: u8 = 28;
    pub const B_CYCLE_JOG_DIST: u8 = 29;
    pub const T_Z_POS: u8 = 30;
    pub const B_SETTINGS: u8 = 31;
    pub const T_MESSAGE_LINE: u8 = 32;
    pub const B_PITCH_005: u8 = 33;
    pub const B_PITCH_05: u8 = 34;
    pub const B_PITCH_5: u8 = 35;
}

pub fn key_for(touch: Touch) -> Option<Key> {
    if touch.page != 0 {
        return None;
    }
    // Jog buttons and backspace need both press and release events.
    match touch.component {
        id::B_BACKSPACE if !touch.pressed => return Some(Key::BackspaceReleased),
        id::B_JOG_L => return Some(Key::Jog { left: true, pressed: touch.pressed }),
        id::B_JOG_R => return Some(Key::Jog { left: false, pressed: touch.pressed }),
        _ if !touch.pressed => return None,
        _ => {}
    }
    Some(match touch.component {
        id::T_ANGLE | id::T_TURNS => Key::ZeroTurns,
        id::B_LEFT_STOP => Key::StopLeft,
        id::B_RIGHT_STOP => Key::StopRight,
        id::B_NUM1 => Key::Digit(1),
        id::B_NUM2 => Key::Digit(2),
        id::B_NUM3 => Key::Digit(3),
        id::B_BACKSPACE => Key::Backspace,
        id::B_NUM4 => Key::Digit(4),
        id::B_NUM5 => Key::Digit(5),
        id::B_NUM6 => Key::Digit(6),
        id::B_NUM7 => Key::Digit(7),
        id::B_NUM8 => Key::Digit(8),
        id::B_NUM9 => Key::Digit(9),
        id::B_NUM0 => Key::Digit(0),
        id::B_NUM_OK => Key::Enter,
        id::B_NUM_PERIOD => Key::Point,
        id::B_TOGGLE_ENGAGED => Key::ToggleEngage,
        id::B_REVERSE_TOGGLE => Key::Reverse,
        id::B_UNITS_TOGGLE => Key::ToggleUnit,
        id::B_PITCH_001 => Key::PitchPreset(10),
        id::B_PITCH_01 => Key::PitchPreset(100),
        id::B_PITCH_1 => Key::PitchPreset(1_000),
        id::B_PITCH_005 => Key::PitchPreset(50),
        id::B_PITCH_05 => Key::PitchPreset(500),
        id::B_PITCH_5 => Key::PitchPreset(5_000),
        id::B_ZERO_Z => Key::ZeroZ,
        id::B_CYCLE_JOG_DIST => Key::JogCycle,
        id::B_SETTINGS => Key::Setup,
        _ => return None,
    })
}

/// The output buffer is too small for the command. Nothing was appended:
/// a half-written command would reach the display as a different one.
#[derive(Debug, PartialEq, Eq)]
pub struct BufferFull;

/// Run `write`, undoing its partial output if it runs out of room.
fn whole<const N: usize>(
    out: &mut heapless::Vec<u8, N>,
    write: impl FnOnce(&mut heapless::Vec<u8, N>) -> Result<(), ()>,
) -> Result<(), BufferFull> {
    let len = out.len();
    write(out).map_err(|()| {
        out.truncate(len);
        BufferFull
    })
}

/// Append each part in turn.
fn parts<const N: usize>(out: &mut heapless::Vec<u8, N>, parts: &[&[u8]]) -> Result<(), BufferFull> {
    whole(out, |out| parts.iter().try_for_each(|p| out.extend_from_slice(p).map_err(|_| ())))
}

/// Append `id.txt="text"` + terminator. Text goes as UTF-8, the encoding of
/// both fonts in the HMI (a font only shows the characters it was generated
/// with). Quotes, backslashes and control characters become `?`.
pub fn encode_text<const N: usize>(out: &mut heapless::Vec<u8, N>, id: &str, text: &str) -> Result<(), BufferFull> {
    whole(out, |out| {
        out.extend_from_slice(id.as_bytes()).map_err(|_| ())?;
        out.extend_from_slice(b".txt=\"").map_err(|_| ())?;
        for c in text.chars() {
            let c = if matches!(c, '"' | '\\') || c.is_control() { '?' } else { c };
            let mut utf8 = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes()).map_err(|_| ())?;
        }
        out.push(b'"').map_err(|_| ())?;
        out.extend_from_slice(&TERM).map_err(|_| ())
    })
}

/// Append `id.attr=value` + terminator, e.g. `bLeftStop.bco=63488`.
pub fn encode_num<const N: usize>(
    out: &mut heapless::Vec<u8, N>,
    id: &str,
    attr: &str,
    value: u32,
) -> Result<(), BufferFull> {
    use core::fmt::Write;
    let mut num: heapless::String<10> = heapless::String::new();
    let _ = write!(num, "{value}");
    parts(out, &[id.as_bytes(), b".", attr.as_bytes(), b"=", num.as_bytes(), &TERM])
}

/// Short beep through the display's speaker.
pub const BEEP: &[u8] = b"play 0,0,0\xFF\xFF\xFF";

/// Append `baud=<rate>` + terminator: switch the display's serial rate.
pub fn encode_baud<const N: usize>(out: &mut heapless::Vec<u8, N>, rate: u32) -> Result<(), BufferFull> {
    use core::fmt::Write;
    let mut num: heapless::String<10> = heapless::String::new();
    let _ = write!(num, "{rate}");
    parts(out, &[b"baud=", num.as_bytes(), &TERM])
}

/// Reload page 0: every component back to its HMI colours, touch enabled.
/// The leading terminator ends any partial command first, e.g. the baud
/// handshake arriving as noise when the display is already at our rate.
pub const RELOAD_PAGE: &[u8] = b"\xFF\xFF\xFFpage 0\xFF\xFF\xFF";

/// Append `tsw id,0|1` + terminator: turn a component's touch events off or on.
pub fn encode_touch<const N: usize>(out: &mut heapless::Vec<u8, N>, id: &str, enabled: bool) -> Result<(), BufferFull> {
    parts(out, &[b"tsw ", id.as_bytes(), if enabled { b",1" } else { b",0" }, &TERM])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_touch_events_and_ignores_noise() {
        let mut p = Parser::default();
        let bytes = [0x88, 0xFF, 0xFF, 0xFF, 0x65, 0x00, 0x13, 0x01, 0xFF, 0xFF, 0xFF];
        let events: Vec<_> = bytes.iter().filter_map(|&b| p.push(b)).collect();
        assert_eq!(events, vec![Touch { page: 0, component: id::B_TOGGLE_ENGAGED, pressed: true }]);
        assert_eq!(key_for(events[0]), Some(Key::ToggleEngage));
    }

    /// Framing as captured from the real display; component ids updated to
    /// the current HMI.
    #[test]
    fn captured_display_trace() {
        let bytes = [
            0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, // power-on
            0x88, 0xFF, 0xFF, 0xFF, // ready
            0x65, 0x00, 0x12, 0x01, 0xFF, 0xFF, 0xFF, // bNumPeriod press
            0x65, 0x00, 0x13, 0x01, 0xFF, 0xFF, 0xFF, // bToggleEngaged press
            0x65, 0x00, 0x1A, 0x01, 0xFF, 0xFF, 0xFF, // bZeroZ press
            0x65, 0x00, 0x1A, 0x00, 0xFF, 0xFF, 0xFF, // bZeroZ release
        ];
        let mut p = Parser::default();
        let keys: Vec<_> = bytes.iter().filter_map(|&b| p.push(b)).map(key_for).collect();
        assert_eq!(keys, [Some(Key::Point), Some(Key::ToggleEngage), Some(Key::ZeroZ), None]);
    }

    #[test]
    fn maps_digits_and_steps() {
        let t = |c| Touch { page: 0, component: c, pressed: true };
        use id::*;
        let digits = [B_NUM0, B_NUM1, B_NUM2, B_NUM3, B_NUM4, B_NUM5, B_NUM6, B_NUM7, B_NUM8, B_NUM9];
        for (d, &c) in digits.iter().enumerate() {
            assert_eq!(key_for(t(c)), Some(Key::Digit(d as u8)));
        }
        let presets = [
            (B_PITCH_001, 10),
            (B_PITCH_005, 50),
            (B_PITCH_01, 100),
            (B_PITCH_05, 500),
            (B_PITCH_1, 1_000),
            (B_PITCH_5, 5_000),
        ];
        for (c, milli) in presets {
            assert_eq!(key_for(t(c)), Some(Key::PitchPreset(milli)));
        }
        assert_eq!(key_for(Touch { pressed: false, ..t(B_NUM0) }), None);
        assert_eq!(key_for(Touch { pressed: false, ..t(B_JOG_R) }), Some(Key::Jog { left: false, pressed: false }));
        assert_eq!(key_for(Touch { pressed: false, ..t(B_BACKSPACE) }), Some(Key::BackspaceReleased));
        assert_eq!(key_for(t(B_CYCLE_JOG_DIST)), Some(Key::JogCycle));
        assert_eq!(key_for(t(B_SETTINGS)), Some(Key::Setup));
        // Static labels and unused components do nothing.
        for c in [T_RPM, T_PITCH, T_Z_POS, T_MESSAGE_LINE, 36] {
            assert_eq!(key_for(t(c)), None);
        }
    }

    #[test]
    fn encodes_number() {
        let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
        encode_num(&mut v, "bLeftStop", "bco", 63488).unwrap();
        assert_eq!(&v[..], b"bLeftStop.bco=63488\xFF\xFF\xFF");
    }

    #[test]
    fn encodes_text() {
        let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
        encode_text(&mut v, "tAngle", "12.5°").unwrap();
        assert_eq!(&v[..], "tAngle.txt=\"12.5°\"".as_bytes().iter().chain(&TERM).copied().collect::<Vec<_>>());
        v.clear();
        encode_text(&mut v, "t", "a\"b\n").unwrap();
        assert_eq!(&v[..], b"t.txt=\"a?b?\"\xFF\xFF\xFF");
        v.clear();
        encode_touch(&mut v, "bJogL", false).unwrap();
        assert_eq!(&v[..], b"tsw bJogL,0\xFF\xFF\xFF");
        v.clear();
        encode_baud(&mut v, 115_200).unwrap();
        assert_eq!(&v[..], b"baud=115200\xFF\xFF\xFF");
    }

    #[test]
    fn a_command_that_does_not_fit_appends_nothing() {
        let mut v: heapless::Vec<u8, 24> = heapless::Vec::new();
        encode_touch(&mut v, "bToggleEngaged", false).unwrap();
        let before = v.clone();
        assert_eq!(encode_num(&mut v, "bToggleEngaged", "bco", 6371), Err(BufferFull));
        assert_eq!(encode_text(&mut v, "bToggleEngaged", "Engage"), Err(BufferFull));
        assert_eq!(v, before);
    }
}
