//! Framing for small records persisted in flash.
//!
//! ```text
//! offset  size  field
//!      0     4  magic     identifies the record kind; erased flash reads 0xFFFFFFFF
//!      4     2  version   payload format version, for migrations
//!      6     2  length    payload length in bytes
//!      8     4  sequence  incremented on every save; the newer valid slot wins
//!     12     4  crc32     over version, length, sequence and payload
//!     16     n  payload
//! ```
//!
//! Each record kind uses two slots (flash sectors) written alternately, so a
//! power cut during a save leaves the previous copy intact. Everything is
//! little-endian.

pub const HEADER_LEN: usize = 16;

/// Why a slot didn't hold a usable record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotError {
    /// Erased flash: nothing was ever written.
    Blank,
    /// Something is there but it isn't a valid record of this kind (random
    /// data, a torn write, bit rot).
    Corrupt,
}

/// A valid record read from a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record<'a> {
    pub version: u16,
    pub sequence: u32,
    pub payload: &'a [u8],
}

/// Result of loading the newest valid record from a pair of slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded<'a> {
    Found {
        record: Record<'a>,
        slot: usize,
    },
    /// Both slots erased.
    Blank,
    /// No valid record and at least one slot held garbage.
    Corrupt,
}

/// Write a record into `out`; returns the number of bytes used.
pub fn encode(magic: u32, version: u16, sequence: u32, payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let total = HEADER_LEN + payload.len();
    if out.len() < total || payload.len() > u16::MAX as usize {
        return None;
    }
    out[0..4].copy_from_slice(&magic.to_le_bytes());
    out[4..6].copy_from_slice(&version.to_le_bytes());
    out[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    out[8..12].copy_from_slice(&sequence.to_le_bytes());
    out[HEADER_LEN..total].copy_from_slice(payload);
    let crc = crc32(&[&out[4..12], payload]);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    Some(total)
}

/// Read and check a record of kind `magic` from the start of `slot`.
pub fn decode(magic: u32, slot: &[u8]) -> Result<Record<'_>, SlotError> {
    if slot.len() < HEADER_LEN {
        return Err(SlotError::Corrupt);
    }
    if slot[..HEADER_LEN].iter().all(|&b| b == 0xFF) {
        return Err(SlotError::Blank);
    }
    let u16_at = |i: usize| u16::from_le_bytes([slot[i], slot[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([slot[i], slot[i + 1], slot[i + 2], slot[i + 3]]);
    if u32_at(0) != magic {
        return Err(SlotError::Corrupt);
    }
    let len = u16_at(6) as usize;
    let payload = slot.get(HEADER_LEN..HEADER_LEN + len).ok_or(SlotError::Corrupt)?;
    if crc32(&[&slot[4..12], payload]) != u32_at(12) {
        return Err(SlotError::Corrupt);
    }
    Ok(Record { version: u16_at(4), sequence: u32_at(8), payload })
}

/// Pick the newest valid record of kind `magic` from two slots.
pub fn load<'a>(magic: u32, slots: [&'a [u8]; 2]) -> Loaded<'a> {
    let a = decode(magic, slots[0]);
    let b = decode(magic, slots[1]);
    match (a, b) {
        // Sequence numbers wrap; the newer one is at most half the range ahead.
        (Ok(x), Ok(y)) if y.sequence.wrapping_sub(x.sequence) as i32 > 0 => Loaded::Found { record: y, slot: 1 },
        (Ok(x), _) => Loaded::Found { record: x, slot: 0 },
        (_, Ok(y)) => Loaded::Found { record: y, slot: 1 },
        (Err(SlotError::Blank), Err(SlotError::Blank)) => Loaded::Blank,
        _ => Loaded::Corrupt,
    }
}

/// Slot and sequence number for the next save, given what `load` found.
pub fn next_slot(loaded: &Loaded) -> (usize, u32) {
    match loaded {
        Loaded::Found { record, slot } => (1 - slot, record.sequence.wrapping_add(1)),
        _ => (0, 1),
    }
}

/// CRC-32 (IEEE 802.3) over several byte slices.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for part in parts {
        for &byte in *part {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

/// Little-endian payload writer.
pub struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
    overflow: bool,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0, overflow: false }
    }

    pub fn bytes(&mut self, b: &[u8]) {
        match self.buf.get_mut(self.pos..self.pos + b.len()) {
            Some(dst) => {
                dst.copy_from_slice(b);
                self.pos += b.len();
            }
            None => self.overflow = true,
        }
    }

    pub fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }

    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    /// Bytes written, or `None` if the buffer was too small.
    pub fn finish(self) -> Option<usize> {
        (!self.overflow).then_some(self.pos)
    }
}

/// Little-endian payload reader. Reading past the end yields `None`.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.buf.get(self.pos..self.pos + N)?;
        self.pos += N;
        b.try_into().ok()
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    pub fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// True if every byte was consumed.
    pub fn done(&self) -> bool {
        self.pos == self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGIC: u32 = u32::from_le_bytes(*b"TEST");

    fn slot(sequence: u32, payload: &[u8]) -> [u8; 64] {
        let mut s = [0xFF; 64];
        encode(MAGIC, 1, sequence, payload, &mut s).unwrap();
        s
    }

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(&[b"123456789"]), 0xCBF4_3926);
        assert_eq!(crc32(&[b"1234", b"56789"]), 0xCBF4_3926);
    }

    #[test]
    fn round_trips() {
        let s = slot(7, b"hello");
        assert_eq!(decode(MAGIC, &s), Ok(Record { version: 1, sequence: 7, payload: b"hello" }));
    }

    #[test]
    fn distinguishes_blank_from_corrupt() {
        assert_eq!(decode(MAGIC, &[0xFF; 64]), Err(SlotError::Blank));
        assert_eq!(decode(MAGIC, &[0x00; 64]), Err(SlotError::Corrupt));
        let mut s = slot(1, b"hello");
        s[HEADER_LEN + 1] ^= 0x01; // flipped bit in payload
        assert_eq!(decode(MAGIC, &s), Err(SlotError::Corrupt));
        let other = u32::from_le_bytes(*b"OTHR");
        assert_eq!(decode(other, &slot(1, b"x")), Err(SlotError::Corrupt));
        let mut s = slot(1, b"hello");
        s[6] = 200; // length beyond the slot
        assert_eq!(decode(MAGIC, &s), Err(SlotError::Corrupt));
    }

    #[test]
    fn newest_valid_slot_wins() {
        let (a, b) = (slot(4, b"old"), slot(5, b"new"));
        assert!(matches!(load(MAGIC, [&a, &b]), Loaded::Found { slot: 1, record } if record.payload == b"new"));
        assert!(matches!(load(MAGIC, [&b, &a]), Loaded::Found { slot: 0, .. }));
        // Wrapped sequence numbers.
        let (a, b) = (slot(u32::MAX, b"old"), slot(0, b"new"));
        assert!(matches!(load(MAGIC, [&a, &b]), Loaded::Found { slot: 1, .. }));
    }

    #[test]
    fn torn_write_falls_back_to_other_slot() {
        let a = slot(4, b"good");
        let mut b = slot(5, b"torn");
        b[HEADER_LEN + 2..].fill(0xFF); // power cut half-way through the payload
        let loaded = load(MAGIC, [&a, &b]);
        assert!(matches!(loaded, Loaded::Found { slot: 0, record } if record.payload == b"good"));
        assert_eq!(next_slot(&loaded), (1, 5));
    }

    #[test]
    fn blank_and_corrupt_pairs() {
        let blank = [0xFF; 64];
        assert_eq!(load(MAGIC, [&blank, &blank]), Loaded::Blank);
        assert_eq!(load(MAGIC, [&blank, &[0x42; 64]]), Loaded::Corrupt);
        assert_eq!(next_slot(&Loaded::Blank), (0, 1));
    }

    #[test]
    fn writer_and_reader() {
        let mut buf = [0u8; 8];
        let mut w = Writer::new(&mut buf);
        w.u32(0xDEAD_BEEF);
        w.u16(7);
        w.bool(true);
        assert_eq!(w.finish(), Some(7));
        let mut r = Reader::new(&buf[..7]);
        assert_eq!((r.u32(), r.u16(), r.bool()), (Some(0xDEAD_BEEF), Some(7), Some(true)));
        assert!(r.done());
        assert_eq!(r.u8(), None);
        let mut small = [0u8; 2];
        let mut w = Writer::new(&mut small);
        w.u32(1);
        assert_eq!(w.finish(), None);
    }
}
