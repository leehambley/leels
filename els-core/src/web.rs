//! The setup web page: minimal HTTP request parsing, the settings form, and
//! applying a submitted form to [`Settings`].

use core::fmt::{self, Write};

use crate::settings::{Field, Kind, Settings, Timing, FIELDS, SECTIONS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub method: Method,
    /// Path without query string.
    pub path: &'a str,
    /// Bytes up to and including the blank line after the headers.
    pub header_len: usize,
    pub content_length: usize,
}

/// Parse the request line and headers once they are complete in `buf`.
/// `None` means more bytes are needed (or the request is malformed).
pub fn parse_request(buf: &[u8]) -> Option<Request<'_>> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = core::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut parts = lines.next()?.split(' ');
    let method = match parts.next()? {
        "GET" | "HEAD" => Method::Get,
        "POST" => Method::Post,
        _ => Method::Other,
    };
    let target = parts.next()?;
    let path = target.split('?').next().unwrap_or(target);
    let mut content_length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok()?;
            }
        }
    }
    Some(Request { method, path, header_len: end + 4, content_length })
}

/// Why a submitted form was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    /// A field is missing, unparseable or out of range.
    Field(&'static str),
    /// All fields parse but the combination is invalid.
    Invalid(&'static str),
}

impl FormError {
    pub fn message(&self) -> &'static str {
        match self {
            FormError::Field(label) | FormError::Invalid(label) => label,
        }
    }
}

/// Apply an `application/x-www-form-urlencoded` body on top of `base`.
/// Every numeric field must be present; unticked checkboxes are absent.
pub fn apply_form(body: &str, base: &Settings, timing: Timing) -> Result<Settings, FormError> {
    let mut s = *base;
    for f in FIELDS {
        if f.kind == Kind::Flag {
            (f.set)(&mut s, 0);
        }
    }
    let mut seen = 0u64;
    for pair in body.split('&') {
        let (key, raw) = pair.split_once('=').unwrap_or((pair, ""));
        let Some((i, f)) = FIELDS.iter().enumerate().find(|(_, f)| f.key == key) else { continue };
        let mut buf = [0u8; 24];
        let value = url_decode(raw, &mut buf).ok_or(FormError::Field(f.label))?.trim();
        let v = match f.kind {
            Kind::Flag => 1,
            Kind::Integer => value.parse::<i64>().map_err(|_| FormError::Field(f.label))?,
            Kind::Decimal4 => parse_decimal4(value).ok_or(FormError::Field(f.label))?,
        };
        if v < f.min || v > f.max {
            return Err(FormError::Field(f.label));
        }
        (f.set)(&mut s, v);
        seen |= 1 << i;
    }
    for (i, f) in FIELDS.iter().enumerate() {
        if f.kind != Kind::Flag && seen & (1 << i) == 0 {
            return Err(FormError::Field(f.label));
        }
    }
    s.validate(timing).map_err(FormError::Invalid)?;
    Ok(s)
}

/// Decode `%XX` escapes and `+` into `buf`.
fn url_decode<'a>(s: &str, buf: &'a mut [u8]) -> Option<&'a str> {
    let bytes = s.as_bytes();
    let (mut i, mut n) = (0, 0);
    while i < bytes.len() {
        let b = match bytes[i] {
            b'+' => b' ',
            b'%' => {
                let hex = core::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                i += 2;
                u8::from_str_radix(hex, 16).ok()?
            }
            b => b,
        };
        *buf.get_mut(n)? = b;
        n += 1;
        i += 1;
    }
    core::str::from_utf8(&buf[..n]).ok()
}

/// "4", "4.0", "3.175", ".5" → value × 10000. At most 4 decimals.
fn parse_decimal4(s: &str) -> Option<i64> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if frac.len() > 4 || (int.is_empty() && frac.is_empty()) {
        return None;
    }
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || !digits(frac) {
        return None;
    }
    let mut v: i64 = if int.is_empty() { 0 } else { int.parse().ok()? };
    for i in 0..4 {
        let d = frac.as_bytes().get(i).map_or(0, |b| (b - b'0') as i64);
        v = v.checked_mul(10)?.checked_add(d)?;
    }
    Some(v)
}

/// Render the settings page. `notice` is shown at the top (e.g. an error).
pub fn render_page(out: &mut impl Write, s: &Settings, notice: Option<&str>) -> fmt::Result {
    out.write_str(PAGE_HEAD)?;
    if let Some(n) = notice {
        write!(out, "<p class=notice>{}</p>", Escape(n))?;
    }
    out.write_str("<form method=post action=/save>")?;
    for section in SECTIONS {
        write!(out, "<fieldset><legend>{section}</legend>")?;
        for f in FIELDS.iter().filter(|f| f.section == section) {
            render_field(out, f, s)?;
        }
        out.write_str("</fieldset>")?;
    }
    out.write_str("<button>Save and restart</button></form></body></html>")
}

fn render_field(out: &mut impl Write, f: &Field, s: &Settings) -> fmt::Result {
    let v = (f.get)(s);
    match f.kind {
        Kind::Flag => write!(
            out,
            "<label class=check><input type=checkbox name={} value=1{}> {}</label>",
            f.key,
            if v != 0 { " checked" } else { "" },
            f.label
        )?,
        Kind::Integer => write!(
            out,
            "<label>{} <small>{}</small><input name={} type=number min={} max={} step=1 value={v} required></label>",
            f.label, f.unit, f.key, f.min, f.max
        )?,
        Kind::Decimal4 => write!(
            out,
            "<label>{} <small>{}</small><input name={} inputmode=decimal pattern=\"[0-9]*\\.?[0-9]{{0,4}}\" value={} required></label>",
            f.label,
            f.unit,
            f.key,
            Decimal4(v)
        )?,
    }
    if !f.help.is_empty() {
        write!(out, "<p class=help>{}</p>", f.help)?;
    }
    Ok(())
}

/// Value of `key` in a urlencoded form body, decoded into `buf`.
pub fn form_field<'b>(body: &str, key: &str, buf: &'b mut [u8]) -> Option<&'b str> {
    let raw = body.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then_some(v)
    })?;
    url_decode(raw, buf).map(str::trim)
}

/// A form field holding mm with up to 4 decimals, as du.
pub fn form_mm(body: &str, key: &str) -> Option<i64> {
    let mut buf = [0u8; 24];
    parse_decimal4(form_field(body, key, &mut buf)?)
}

// ---------------------------------------------------------------------------
// Backlash wizard
//
// With a dial indicator on the carriage: move towards +Z (left) to push the
// nut against one side of the screw, zero the indicator, then reverse by
// PROBE. The carriage should move PROBE; whatever is missing is slack the
// compensation in use doesn't cover yet. Each round runs with the estimate
// so far switched on, so a second round checks (and refines) the first.
// ---------------------------------------------------------------------------

/// Move towards +Z (left) before each measurement, to take up the slack.
pub const WIZARD_TAKEUP_DU: i64 = 10_000;
/// Reversal the indicator measures, towards -Z (right).
pub const WIZARD_PROBE_DU: i64 = 5_000;
/// Largest backlash accepted (the setting's limit), du.
pub const MAX_BACKLASH_DU: i64 = 20_000;
/// A round within this of the expected travel means the value is right.
pub const WIZARD_GOOD_DU: i64 = 50;

/// Backlash estimate after a round measured with `compensation_du` on: the
/// indicator read `reading_du` of an expected [`WIZARD_PROBE_DU`].
pub fn refine_backlash(compensation_du: i64, reading_du: i64) -> i64 {
    (compensation_du + WIZARD_PROBE_DU - reading_du).clamp(0, MAX_BACKLASH_DU)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardStep {
    /// What's needed; `saved_du` is the backlash setting in use.
    Intro { saved_du: i64 },
    /// Carriage moved left with `compensation_du` on: zero the indicator.
    Zero { compensation_du: i64 },
    /// Carriage reversed: enter the indicator reading.
    Read { compensation_du: i64 },
    /// Measured with `compensation_du` on; the new estimate is `backlash_du`.
    Result { compensation_du: i64, reading_du: i64, backlash_du: i64 },
}

pub fn render_backlash(out: &mut impl Write, step: WizardStep, notice: Option<&str>) -> fmt::Result {
    out.write_str(PAGE_HEAD)?;
    out.write_str("<h2>Measure backlash</h2>")?;
    if let Some(n) = notice {
        write!(out, "<p class=notice>{}</p>", Escape(n))?;
    }
    let takeup = Decimal4(WIZARD_TAKEUP_DU);
    let probe = Decimal4(WIZARD_PROBE_DU);
    let button = |out: &mut dyn Write, action: &str, b: i64, label: fmt::Arguments| {
        write!(
            out,
            "<form method=post action=/backlash/{action}><input type=hidden name=b value={}><button>{label}</button></form>",
            Decimal4(b)
        )
    };
    match step {
        WizardStep::Intro { saved_du } => {
            write!(
                out,
                "<ol><li>Fix a dial indicator to the bed so it reads the carriage moving along the bed.</li>\
                 <li>Keep hands clear: the carriage moves {takeup} mm left, then {probe} mm right, \
                 like the JOG buttons.</li></ol>\
                 <p class=help>Compensation is off for the first round. The saved value is {} mm. \
                 Nothing is saved until the last step.</p>",
                Decimal4(saved_du)
            )?;
            button(out, "takeup", 0, format_args!("Start: move {takeup} mm left"))?;
        }
        WizardStep::Zero { compensation_du } => {
            write!(out, "<p>The slack is taken up. <b>Zero the indicator</b>, then reverse.</p>")?;
            button(out, "probe", compensation_du, format_args!("Move {probe} mm right"))?;
        }
        WizardStep::Read { compensation_du } => {
            write!(
                out,
                "<form method=post action=/backlash/result><input type=hidden name=b value={}>\
                 <label>How far did the indicator move? <small>mm</small>\
                 <input name=reading inputmode=decimal pattern=\"[0-9]*\\.?[0-9]{{0,4}}\" required autofocus></label>\
                 <p class=help>The carriage was told to move {probe} mm.</p><button>Calculate</button></form>",
                Decimal4(compensation_du)
            )?;
        }
        WizardStep::Result { compensation_du, reading_du, backlash_du } => {
            let short = WIZARD_PROBE_DU - reading_du;
            write!(
                out,
                "<p>Moved {} of {probe} mm with {} mm compensation.</p><p><b>Backlash: {} mm</b></p>",
                Decimal4(reading_du),
                Decimal4(compensation_du),
                Decimal4(backlash_du)
            )?;
            if compensation_du > 0 && short.abs() <= WIZARD_GOOD_DU {
                write!(
                    out,
                    "<p class=help>Within {} mm of the target: this value is right.</p>",
                    Decimal4(WIZARD_GOOD_DU)
                )?;
            } else {
                out.write_str("<p class=help>Check it: another round runs with this value switched on.</p>")?;
            }
            button(out, "takeup", backlash_du, format_args!("Check with {} mm", Decimal4(backlash_du)))?;
            out.write_str("<p></p>")?;
            button(out, "save", backlash_du, format_args!("Save {} mm and restart", Decimal4(backlash_du)))?;
        }
    }
    out.write_str("<p><a href=/>Back to settings</a> (nothing saved)</p></body></html>")
}

/// Page shown after a successful save.
pub const SAVED_PAGE: &str = concat!(
    "<!doctype html><html><head><meta charset=utf-8>",
    "<meta name=viewport content=\"width=device-width,initial-scale=1\"><title>ELS setup</title></head>",
    "<body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:2rem auto;padding:0 1rem\">",
    "<h1>Saved</h1><p>The controller is restarting with the new settings. WiFi is now off.</p></body></html>"
);

const PAGE_HEAD: &str = concat!(
    "<!doctype html><html><head><meta charset=utf-8>",
    "<meta name=viewport content=\"width=device-width,initial-scale=1\"><title>ELS setup</title><style>",
    "body{font-family:system-ui,sans-serif;max-width:32rem;margin:0 auto;padding:1rem;background:#f4f6f8;color:#222}",
    "fieldset{background:#fff;border:1px solid #c9d0d6;border-radius:6px;margin:0 0 1rem;padding:.5rem 1rem}",
    "legend{font-weight:700;padding:0 .3rem}label{display:block;font-weight:600;margin-top:.8rem}",
    "small{color:#5d6770;font-weight:400}input{display:block;width:100%;box-sizing:border-box;padding:.5rem;font:inherit;margin-top:.2rem}",
    "label.check input{display:inline;width:auto}.help{color:#5d6770;font-size:.85rem;margin:.2rem 0 0}",
    ".notice{background:#fff3e0;border:1px solid #e0b46a;border-radius:6px;padding:.6rem}",
    "button{width:100%;padding:.8rem;font:inherit;font-weight:700;color:#fff;background:#217346;border:0;border-radius:6px}",
    "@media(prefers-color-scheme:dark){body{background:#15191c;color:#e6e6e6}fieldset{background:#1e2428;border-color:#3a4248}",
    "small,.help{color:#9aa5ad}input{background:#2a3136;color:#e6e6e6;border:1px solid #3a4248}.notice{background:#3a2e1a;border-color:#8a6a2a}}",
    "</style></head><body><h1>ELS setup</h1>"
);

/// Formats a ×10000 value with 4 decimals.
struct Decimal4(i64);

impl fmt::Display for Decimal4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:04}", self.0 / 10_000, self.0 % 10_000)
    }
}

/// HTML-escapes text.
struct Escape<'a>(&'a str);

impl fmt::Display for Escape<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.0.chars() {
            match c {
                '<' => f.write_str("&lt;")?,
                '>' => f.write_str("&gt;")?,
                '&' => f.write_str("&amp;")?,
                '"' => f.write_str("&quot;")?,
                c => f.write_char(c)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::tests::{DEFAULTS, TIMING};

    /// Serialise settings the way a browser submits the rendered form.
    fn form_body(s: &Settings) -> String {
        FIELDS
            .iter()
            .filter_map(|f| {
                let v = (f.get)(s);
                match f.kind {
                    Kind::Flag if v == 0 => None,
                    Kind::Flag => Some(format!("{}=1", f.key)),
                    Kind::Integer => Some(format!("{}={v}", f.key)),
                    Kind::Decimal4 => Some(format!("{}={}", f.key, Decimal4(v))),
                }
            })
            .collect::<Vec<_>>()
            .join("&")
    }

    #[test]
    fn parses_requests() {
        let r = parse_request(b"POST /save?x=1 HTTP/1.1\r\nHost: a\r\nContent-Length: 12\r\n\r\nbody").unwrap();
        assert_eq!(r, Request { method: Method::Post, path: "/save", header_len: 56, content_length: 12 });
        assert_eq!(parse_request(b"GET / HTTP/1.1\r\nHost: a\r\n"), None);
        assert_eq!(parse_request(b"GET /generate_204 HTTP/1.1\r\n\r\n").unwrap().path, "/generate_204");
    }

    #[test]
    fn form_round_trips() {
        let mut s = DEFAULTS;
        s.screw_du = 31_750; // 8 TPI
        s.invert_dir = true;
        s.step_active_low = false;
        let got = apply_form(&form_body(&s), &DEFAULTS, TIMING).unwrap();
        assert_eq!(got, s);
    }

    #[test]
    fn decimals_and_escapes() {
        assert_eq!(parse_decimal4("3.175"), Some(31_750));
        assert_eq!(parse_decimal4(".5"), Some(5_000));
        assert_eq!(parse_decimal4("4"), Some(40_000));
        assert_eq!(parse_decimal4("1.23456"), None);
        assert_eq!(parse_decimal4("-1"), None);
        let body = form_body(&DEFAULTS).replace("screw_pitch=4.0000", "screw_pitch=+%33%2E175");
        assert_eq!(apply_form(&body, &DEFAULTS, TIMING).unwrap().screw_du, 31_750);
    }

    #[test]
    fn rejects_missing_out_of_range_and_inconsistent() {
        let body = form_body(&DEFAULTS).replace("motor_steps=800&", "");
        assert_eq!(apply_form(&body, &DEFAULTS, TIMING), Err(FormError::Field("Motor steps per screw turn")));
        let body = form_body(&DEFAULTS).replace("motor_steps=800", "motor_steps=0");
        assert!(matches!(apply_form(&body, &DEFAULTS, TIMING), Err(FormError::Field(_))));
        let body = form_body(&DEFAULTS).replace("speed_start=800", "speed_start=31000");
        assert!(matches!(apply_form(&body, &DEFAULTS, TIMING), Err(FormError::Invalid(_))));
    }

    #[test]
    fn backlash_wizard_refines_and_renders() {
        // First round, no compensation: 0.35 of 0.5 mm means 0.15 mm slack.
        assert_eq!(refine_backlash(0, 3_500), 1_500);
        // Check round with 0.15 on reads 0.49: 0.01 mm still missing.
        assert_eq!(refine_backlash(1_500, 4_900), 1_600);
        // Overshoot lowers it; never below 0 or above the limit.
        assert_eq!(refine_backlash(1_600, 5_100), 1_500);
        assert_eq!(refine_backlash(0, 9_000), 0);
        assert_eq!(refine_backlash(0, 0), 5_000);
        assert_eq!(refine_backlash(19_000, 0), MAX_BACKLASH_DU);

        let page = |step| {
            let mut p = String::new();
            render_backlash(&mut p, step, None).unwrap();
            p
        };
        assert!(page(WizardStep::Intro { saved_du: 700 }).contains("The saved value is 0.0700 mm"));
        assert!(page(WizardStep::Zero { compensation_du: 1_500 })
            .contains("action=/backlash/probe><input type=hidden name=b value=0.1500>"));
        assert!(page(WizardStep::Read { compensation_du: 0 }).contains("name=reading"));
        let done = page(WizardStep::Result { compensation_du: 1_500, reading_du: 4_990, backlash_du: 1_510 });
        assert!(done.contains("<b>Backlash: 0.1510 mm</b>"));
        assert!(done.contains("this value is right"));
        assert!(done.contains("action=/backlash/save><input type=hidden name=b value=0.1510>"));
        assert!(
            !page(WizardStep::Result { compensation_du: 0, reading_du: 4_990, backlash_du: 10 }).contains("is right")
        );
    }

    #[test]
    fn form_fields() {
        assert_eq!(form_mm("b=0.1500&reading=0.49", "b"), Some(1_500));
        assert_eq!(form_mm("b=0.1500&reading=0.49", "reading"), Some(4_900));
        assert_eq!(form_mm("reading=%2E5", "reading"), Some(5_000));
        assert_eq!(form_mm("b=x", "b"), None);
        assert_eq!(form_mm("", "b"), None);
    }

    #[test]
    fn renders_every_field_with_its_value() {
        let mut page = String::new();
        render_page(&mut page, &DEFAULTS, Some("<bad>")).unwrap();
        for f in FIELDS {
            assert!(page.contains(&format!("name={}", f.key)), "{}", f.key);
        }
        assert!(page.contains("name=screw_pitch inputmode=decimal pattern=\"[0-9]*\\.?[0-9]{0,4}\" value=4.0000"));
        assert!(page.contains("name=step_active_low value=1 checked"));
        assert!(page.contains("&lt;bad&gt;"));
        assert!(page.len() < 12_000, "page is {} bytes", page.len());
    }
}
