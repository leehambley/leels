//! Hardware-in-the-loop test support (`--features hil`), driven by the host
//! tests in `hil/`. The host types Nextion touch events into USB-Serial-JTAG
//! and reads the display commands back from the log, so everything from the
//! touch parser to the STEP pin runs as it does with a real display.
//!
//! STEP pulses are counted back off the pin by a second PCNT unit (connected
//! inside the chip, no wiring) and reported with the planned position, so the
//! tests can check that every planned step reached the pin.

use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::Timer;
use esp_hal::gpio::interconnect::InputSignal;
use esp_hal::pcnt::{channel, unit::Unit};
use esp_hal::usb_serial_jtag::UsbSerialJtagRx;
use esp_hal::Async;
use esp_println::println;

use els_core::settings::Settings;
use els_core::spindle::PCNT_LIMIT;

/// (planned position, pulses seen on the STEP pin), both signed steps.
static STEPS: Mutex<CriticalSectionRawMutex, Cell<(i64, i64)>> = Mutex::new(Cell::new((0, 0)));

/// Counts STEP pulses, signed by the DIR level at each pulse.
pub struct StepCounter {
    unit: Unit<'static, 1>,
    last: i32,
    pulses: i64,
}

impl StepCounter {
    pub fn new(unit: Unit<'static, 1>, step: InputSignal<'static>, dir: InputSignal<'static>, s: &Settings) -> Self {
        unit.set_low_limit(Some(-(PCNT_LIMIT as i16))).unwrap();
        unit.set_high_limit(Some(PCNT_LIMIT as i16)).unwrap();
        unit.clear();
        let ch = &unit.channel0;
        ch.set_edge_signal(step);
        ch.set_ctrl_signal(dir);
        // One count per pulse, on its leading edge.
        let (hold, count) = (channel::EdgeMode::Hold, channel::EdgeMode::Increment);
        if s.step_active_low {
            ch.set_input_mode(count, hold);
        } else {
            ch.set_input_mode(hold, count);
        }
        // DIR level that means a positive move (see motion_task).
        let (keep, reverse) = (channel::CtrlMode::Keep, channel::CtrlMode::Reverse);
        if s.invert_dir {
            ch.set_ctrl_mode(keep, reverse);
        } else {
            ch.set_ctrl_mode(reverse, keep);
        }
        unit.resume();
        Self { unit, last: 0, pulses: 0 }
    }

    /// Called every segment, far more often than the counter can wrap.
    pub fn poll(&mut self, planned: i64) {
        let raw = self.unit.value() as i32;
        let mut delta = raw - self.last;
        // The unit resets to 0 on reaching either limit.
        if delta > PCNT_LIMIT / 2 {
            delta -= PCNT_LIMIT;
        } else if delta < -PCNT_LIMIT / 2 {
            delta += PCNT_LIMIT;
        }
        self.last = raw;
        self.pulses += delta as i64;
        STEPS.lock(|c| c.set((planned, self.pulses)));
    }
}

/// Host → display input: bytes typed into USB go where the Nextion's would.
#[embassy_executor::task]
pub async fn usb_rx_task(mut rx: UsbSerialJtagRx<'static, Async>) {
    let mut buf = [0u8; 64];
    loop {
        match embedded_io_async::Read::read(&mut rx, &mut buf).await {
            Ok(n) => super::NEXTION_RX.write_all(&buf[..n]).await,
            Err(e) => println!("hil: usb rx error: {:?}", e),
        }
    }
}

/// Log `hil: steps planned=<n> pulses=<n>` whenever either changes. The
/// planned count lags the pin by up to one segment while moving.
#[embassy_executor::task]
pub async fn report_task() {
    let mut shown = (i64::MIN, i64::MIN);
    loop {
        let now = STEPS.lock(|c| c.get());
        if now != shown {
            println!("hil: steps planned={} pulses={}", now.0, now.1);
            shown = now;
        }
        Timer::after_millis(20).await;
    }
}
