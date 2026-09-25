//! Settings and operator state in flash.
//!
//! Uses the first four 4 KB sectors of the partition table's NVS data
//! partition (as raw flash, not ESP-IDF's NVS format): two alternating slots
//! for settings, two for operator state. Framing, validation and slot
//! selection are in `els_core::record`.

use els_core::record::{self, Loaded};
use els_core::settings::{OperatorState, Origin, Settings, Timing, MAX_RECORD_LEN, SETTINGS_MAGIC, STATE_MAGIC};
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

const SECTOR: u32 = 4096;
const SETTINGS_SLOTS: [u32; 2] = [0, SECTOR];
const STATE_SLOTS: [u32; 2] = [2 * SECTOR, 3 * SECTOR];

pub struct Store {
    flash: FlashStorage<'static>,
    base: u32,
    /// (slot, sequence) for the next save of each record kind.
    next_settings: (usize, u32),
    next_state: (usize, u32),
}

impl Store {
    pub fn new(flash: FLASH<'static>) -> Self {
        let mut flash = FlashStorage::new(flash);
        let mut table = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
        let pt = partitions::read_partition_table(&mut flash, &mut table).expect("failed to read partition table");
        let nvs = pt
            .find_partition(partitions::PartitionType::Data(partitions::DataPartitionSubType::Nvs))
            .expect("partition table read error")
            .expect("no NVS data partition");
        assert!(nvs.len() >= 4 * SECTOR, "NVS partition too small");
        Self { flash, base: nvs.offset(), next_settings: (0, 1), next_state: (0, 1) }
    }

    pub fn load_settings(&mut self, defaults: Settings, timing: Timing) -> (Settings, Origin) {
        let mut a = [0u8; MAX_RECORD_LEN];
        let mut b = [0u8; MAX_RECORD_LEN];
        self.read_slots(SETTINGS_SLOTS, &mut a, &mut b);
        let loaded = record::load(SETTINGS_MAGIC, [&a, &b]);
        self.next_settings = record::next_slot(&loaded);
        Settings::from_loaded(&loaded, defaults, timing)
    }

    pub fn load_state(&mut self, max_pitch_du: i64) -> Option<OperatorState> {
        let mut a = [0u8; MAX_RECORD_LEN];
        let mut b = [0u8; MAX_RECORD_LEN];
        self.read_slots(STATE_SLOTS, &mut a, &mut b);
        let loaded: Loaded = record::load(STATE_MAGIC, [&a, &b]);
        self.next_state = record::next_slot(&loaded);
        OperatorState::from_loaded(&loaded, max_pitch_du)
    }

    /// Blocks the CPU for tens of milliseconds: only call while the carriage is idle.
    pub fn save_settings(&mut self, s: &Settings) -> Result<(), &'static str> {
        let (slot, seq) = self.next_settings;
        let mut buf = [0xFFu8; MAX_RECORD_LEN];
        let len = s.encode(seq, &mut buf).ok_or("settings too large")?;
        self.write(SETTINGS_SLOTS[slot], &buf[..len])?;
        self.next_settings = (1 - slot, seq.wrapping_add(1));
        Ok(())
    }

    /// Blocks the CPU for tens of milliseconds: only call while the carriage is idle.
    pub fn save_state(&mut self, st: &OperatorState) -> Result<(), &'static str> {
        let (slot, seq) = self.next_state;
        let mut buf = [0xFFu8; MAX_RECORD_LEN];
        let len = st.encode(seq, &mut buf).ok_or("state too large")?;
        self.write(STATE_SLOTS[slot], &buf[..len])?;
        self.next_state = (1 - slot, seq.wrapping_add(1));
        Ok(())
    }

    fn read_slots(&mut self, slots: [u32; 2], a: &mut [u8], b: &mut [u8]) {
        // A failed read looks like garbage and is treated as corrupt.
        if self.flash.read(self.base + slots[0], a).is_err() {
            a.fill(0);
        }
        if self.flash.read(self.base + slots[1], b).is_err() {
            b.fill(0);
        }
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), &'static str> {
        // `Storage::write` does read-modify-erase-write of the whole sector.
        self.flash.write(self.base + offset, bytes).map_err(|_| "flash write failed")
    }
}
