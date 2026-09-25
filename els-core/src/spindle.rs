//! Spindle position tracking from a wrapping hardware pulse counter.

/// The PCNT unit is configured with limits of +-`PCNT_LIMIT`; on reaching a
/// limit the hardware resets the counter to 0.
pub const PCNT_LIMIT: i32 = 30_000;

#[derive(Debug, Default)]
pub struct Spindle {
    last_raw: i32,
    /// Raw accumulated position in counts.
    pub pos: i64,
    /// Position with encoder backlash removed: only follows `pos` backwards
    /// once it moved more than `backlash` counts in reverse. This is what the
    /// gearbox follows, so encoder jitter doesn't wiggle the carriage.
    pub avg: i64,
    backlash: i64,
}

impl Spindle {
    pub fn new(backlash: i64) -> Self {
        Self { backlash, ..Default::default() }
    }

    /// Feed the current raw counter value; returns the counts moved since the
    /// previous call. Must be called at least once per `PCNT_LIMIT / 2` counts.
    pub fn feed(&mut self, raw: i16) -> i64 {
        let raw = raw as i32;
        let mut delta = raw - self.last_raw;
        if delta > PCNT_LIMIT / 2 {
            delta -= PCNT_LIMIT;
        } else if delta < -PCNT_LIMIT / 2 {
            delta += PCNT_LIMIT;
        }
        self.last_raw = raw;
        let delta = delta as i64;
        self.pos += delta;
        if self.pos > self.avg {
            self.avg = self.pos;
        } else if self.pos < self.avg - self.backlash {
            self.avg = self.pos + self.backlash;
        }
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_wrap_both_ways() {
        let mut s = Spindle::new(0);
        s.feed(10_000);
        s.feed(20_000);
        s.feed(29_990);
        assert_eq!(s.pos, 29_990);
        s.feed(5); // hit +30000, reset to 0, then +5
        assert_eq!(s.pos, 30_005);
        s.feed(-10);
        assert_eq!(s.pos, 29_990);

        let mut s = Spindle::new(0);
        for raw in [-10_000, -20_000, -29_995] {
            s.feed(raw);
        }
        assert_eq!(s.pos, -29_995);
        s.feed(-3); // hit -30000, reset to 0, then -3
        assert_eq!(s.pos, -30_003);
    }

    #[test]
    fn backlash_filters_small_reversals() {
        let mut s = Spindle::new(3);
        s.feed(100);
        s.feed(98);
        assert_eq!(s.avg, 100);
        s.feed(96);
        assert_eq!(s.avg, 99);
        s.feed(101);
        assert_eq!(s.avg, 101);
    }
}
