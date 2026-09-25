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

/// Component ids on page 0 of the HMI. Existing NanoEls H5 ids are kept where
/// the function is unchanged; 51+ are new buttons. See docs/nextion.md.
pub fn key_for(touch: Touch) -> Option<Key> {
    if touch.page != 0 || !touch.pressed {
        return None;
    }
    Some(match touch.component {
        3 | 23 => Key::Disengage, // bStatus, bOff
        5 => Key::Reverse,        // bReverse
        6 => Key::ToggleUnit,     // bMeasure
        7 => Key::CycleStep,      // bStep / tStepVal
        9 | 10 => Key::ZeroTurns, // tTurns, tAngle
        21 => Key::ZeroZ,         // bZ0
        24 => Key::Backspace,
        25 => Key::Engage, // bOn
        c @ 26..=35 => Key::Digit(c - 26),
        40 => Key::StopLeft,
        41 => Key::StopRight,
        42 => Key::Plus,
        43 => Key::Minus,
        51 => Key::Enter,
        52 => Key::Point,
        c @ 53..=57 => Key::StepSize(c - 53),
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

/// Short beep through the display's speaker.
pub const BEEP: &[u8] = b"play 0,0,0\xFF\xFF\xFF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_touch_events_and_ignores_noise() {
        let mut p = Parser::default();
        let bytes = [0x88, 0xFF, 0xFF, 0xFF, 0x65, 0x00, 0x19, 0x01, 0xFF, 0xFF, 0xFF];
        let events: Vec<_> = bytes.iter().filter_map(|&b| p.push(b)).collect();
        assert_eq!(events, vec![Touch { page: 0, component: 25, pressed: true }]);
        assert_eq!(key_for(events[0]), Some(Key::Engage));
    }

    #[test]
    fn maps_digits_and_steps() {
        let t = |c| Touch { page: 0, component: c, pressed: true };
        assert_eq!(key_for(t(26)), Some(Key::Digit(0)));
        assert_eq!(key_for(t(35)), Some(Key::Digit(9)));
        assert_eq!(key_for(t(57)), Some(Key::StepSize(4)));
        assert_eq!(key_for(Touch { pressed: false, ..t(26) }), None);
    }

    #[test]
    fn encodes_text() {
        let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
        encode_text(&mut v, "tAngleVal", "12.5°").unwrap();
        assert_eq!(&v[..], b"tAngleVal.txt=\"12.5\xDF\"\xFF\xFF\xFF");
    }
}
