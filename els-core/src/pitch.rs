//! Display units and the pitch value the operator works with.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Mm,
    Inch,
}

impl Unit {
    /// du in one thousandth of this unit (0.001mm = 10du, 0.001" = 254du).
    pub const fn du_per_milli(self) -> i64 {
        match self {
            Unit::Mm => 10,
            Unit::Inch => 254,
        }
    }

    pub const fn toggled(self) -> Unit {
        match self {
            Unit::Mm => Unit::Inch,
            Unit::Inch => Unit::Mm,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Unit::Mm => "MM",
            Unit::Inch => "IN",
        }
    }
}

/// Pitch increments selectable with the step buttons, in thousandths of the
/// current unit: .001, .01, .1, 1, 10.
pub const STEP_SIZES_MILLI: [u32; 5] = [1, 10, 100, 1_000, 10_000];

/// Signed pitch expressed as a magnitude in thousandths of `unit` plus a
/// direction. Toggling the unit keeps the number and changes its meaning
/// (0.100mm becomes 0.100").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pitch {
    pub magnitude_milli: u32,
    pub reversed: bool,
    pub unit: Unit,
}

impl Pitch {
    /// Signed pitch in du per spindle revolution.
    pub fn du(&self) -> i64 {
        let du = self.magnitude_milli as i64 * self.unit.du_per_milli();
        if self.reversed {
            -du
        } else {
            du
        }
    }

    /// Signed thousandths of the unit, as shown on screen.
    pub fn signed_milli(&self) -> i64 {
        if self.reversed {
            -(self.magnitude_milli as i64)
        } else {
            self.magnitude_milli as i64
        }
    }
}

/// Parse an operator entry such as "1.25", ".5" or "12" into thousandths of the
/// unit. At most three decimals; anything else is rejected.
pub fn parse_entry_milli(s: &str) -> Option<u32> {
    if s.is_empty() || s == "." {
        return None;
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if frac_part.len() > 3 || frac_part.contains('.') {
        return None;
    }
    let mut value: u32 = 0;
    for c in int_part.bytes().chain(frac_part.bytes()) {
        if !c.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((c - b'0') as u32)?;
    }
    for _ in frac_part.len()..3 {
        value = value.checked_mul(10)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn du_conversion() {
        let p = Pitch { magnitude_milli: 1_000, reversed: false, unit: Unit::Mm };
        assert_eq!(p.du(), 10_000);
        let p = Pitch { unit: Unit::Inch, ..p };
        assert_eq!(p.du(), 254_000);
        let p = Pitch { reversed: true, ..p };
        assert_eq!(p.du(), -254_000);
    }

    #[test]
    fn parses_entries() {
        assert_eq!(parse_entry_milli("1.25"), Some(1_250));
        assert_eq!(parse_entry_milli(".5"), Some(500));
        assert_eq!(parse_entry_milli("12"), Some(12_000));
        assert_eq!(parse_entry_milli("0.001"), Some(1));
        assert_eq!(parse_entry_milli("3."), Some(3_000));
        assert_eq!(parse_entry_milli("0.0001"), None);
        assert_eq!(parse_entry_milli("."), None);
        assert_eq!(parse_entry_milli(""), None);
        assert_eq!(parse_entry_milli("1.2.3"), None);
    }
}
