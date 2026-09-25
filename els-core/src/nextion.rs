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

/// Component ids on page 0 of the Lee-LS HMI. See docs/nextion.md.
pub fn key_for(touch: Touch) -> Option<Key> {
    if touch.page != 0 {
        return None;
    }
    // Jog buttons and backspace need both press and release events.
    match touch.component {
        10 if !touch.pressed => return Some(Key::BackspaceReleased), // bBackspace hold
        29 => return Some(Key::Jog { left: true, pressed: touch.pressed }), // bJogL
        30 => return Some(Key::Jog { left: false, pressed: touch.pressed }), // bJogR
        _ if !touch.pressed => return None,
        _ => {}
    }
    Some(match touch.component {
        3 | 4 => Key::ZeroTurns, // tAngle, tTurns
        5 => Key::StopLeft,      // bLeftStop
        6 => Key::StopRight,     // bRightStop
        7 => Key::Digit(1),      // bNum1
        8 => Key::Digit(2),
        9 => Key::Digit(3),
        10 => Key::Backspace, // bBackspace
        11 => Key::Digit(4),
        12 => Key::Digit(5),
        13 => Key::Digit(6),
        14 => Key::Digit(7),
        15 => Key::Digit(8),
        16 => Key::Digit(9),
        17 => Key::Digit(0),
        18 => Key::Enter,              // bNumOK
        19 => Key::Point,              // bNumPeriod
        20 => Key::ToggleEngage,       // bToggleEngaged
        21 => Key::Reverse,            // bReverseToggle
        22 => Key::ToggleUnit,         // bUnitsToggle
        25 => Key::PitchPreset(10),    // bPitch001 "0.01"
        26 => Key::PitchPreset(100),   // bPitch01 "0.1"
        27 => Key::PitchPreset(1_000), // bPitch1 "1.0"
        28 => Key::ZeroZ,              // bZeroZ
        31 => Key::JogCycle,           // bCycleJogDist
        33 => Key::Setup,              // bSettings
        _ => return None,
    })
}

/// The output buffer is too small for the command.
#[derive(Debug, PartialEq, Eq)]
pub struct BufferFull;

/// Append `id.txt="text"` + terminator. `°` is sent as Nextion's 0xDF glyph;
/// other non-ASCII characters and quotes become `?`.
pub fn encode_text<const N: usize>(out: &mut heapless::Vec<u8, N>, id: &str, text: &str) -> Result<(), BufferFull> {
    out.extend_from_slice(id.as_bytes()).map_err(|_| BufferFull)?;
    out.extend_from_slice(b".txt=\"").map_err(|_| BufferFull)?;
    for c in text.chars() {
        let b = match c {
            '°' => 0xDF,
            '"' | '\\' => b'?',
            c if c.is_ascii() && !c.is_ascii_control() => c as u8,
            _ => b'?',
        };
        out.push(b).map_err(|_| BufferFull)?;
    }
    out.push(b'"').map_err(|_| BufferFull)?;
    out.extend_from_slice(&TERM).map_err(|_| BufferFull)
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
    for part in [id.as_bytes(), b".", attr.as_bytes(), b"=", num.as_bytes(), &TERM] {
        out.extend_from_slice(part).map_err(|_| BufferFull)?;
    }
    Ok(())
}

/// Short beep through the display's speaker.
pub const BEEP: &[u8] = b"play 0,0,0\xFF\xFF\xFF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_touch_events_and_ignores_noise() {
        let mut p = Parser::default();
        let bytes = [0x88, 0xFF, 0xFF, 0xFF, 0x65, 0x00, 0x14, 0x01, 0xFF, 0xFF, 0xFF];
        let events: Vec<_> = bytes.iter().filter_map(|&b| p.push(b)).collect();
        assert_eq!(events, vec![Touch { page: 0, component: 20, pressed: true }]);
        assert_eq!(key_for(events[0]), Some(Key::ToggleEngage));
    }

    /// Bytes captured from the real display on the Lee-LS HMI.
    #[test]
    fn captured_display_trace() {
        let bytes = [
            0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, // power-on
            0x88, 0xFF, 0xFF, 0xFF, // ready
            0x65, 0x00, 0x13, 0x01, 0xFF, 0xFF, 0xFF, // bNumPeriod press
            0x65, 0x00, 0x14, 0x01, 0xFF, 0xFF, 0xFF, // bToggleEngaged press
            0x65, 0x00, 0x1C, 0x01, 0xFF, 0xFF, 0xFF, // bZeroZ press
            0x65, 0x00, 0x1C, 0x00, 0xFF, 0xFF, 0xFF, // bZeroZ release
        ];
        let mut p = Parser::default();
        let keys: Vec<_> = bytes.iter().filter_map(|&b| p.push(b)).map(key_for).collect();
        assert_eq!(keys, [Some(Key::Point), Some(Key::ToggleEngage), Some(Key::ZeroZ), None]);
    }

    #[test]
    fn maps_digits_and_steps() {
        let t = |c| Touch { page: 0, component: c, pressed: true };
        let digits = [17, 7, 8, 9, 11, 12, 13, 14, 15, 16];
        for (d, &c) in digits.iter().enumerate() {
            assert_eq!(key_for(t(c)), Some(Key::Digit(d as u8)));
        }
        assert_eq!(key_for(t(27)), Some(Key::PitchPreset(1_000)));
        assert_eq!(key_for(Touch { pressed: false, ..t(17) }), None);
        assert_eq!(key_for(Touch { pressed: false, ..t(30) }), Some(Key::Jog { left: false, pressed: false }));
        assert_eq!(key_for(t(31)), Some(Key::JogCycle));
        // Static labels and unused ids do nothing.
        assert_eq!(key_for(t(1)), None);
        assert_eq!(key_for(t(24)), None);
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
        assert_eq!(&v[..], b"tAngle.txt=\"12.5\xDF\"\xFF\xFF\xFF");
    }
}
