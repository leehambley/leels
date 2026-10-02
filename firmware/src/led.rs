//! Proof of life on the DevKitC-1's onboard WS2812 RGB LED (GPIO8), driven by
//! RMT channel 1 (channel 0 is STEP).
//!
//! Normal build: a slow rainbow from a thread-executor task for as long as the
//! firmware runs; if the firmware hangs, the colour freezes. Breadboard build:
//! GPIO8 is DIR there, so the rainbow only plays for a moment at boot, with
//! the buffer's outputs held off, before the pin becomes DIR.

use embassy_time::{Instant, Timer};
use esp_hal::gpio::Level;
use esp_hal::rmt::{self, PulseCode, TxChannelConfig};
use esp_hal::Async;

pub type LedChannel<'a> = rmt::Channel<'a, Async, rmt::Tx>;

// WS2812 bit timings in 12.5 ns ticks (80 MHz RMT source, divider 1).
const T0H: u16 = 32; // 0.40 µs
const T0L: u16 = 68; // 0.85 µs
const T1H: u16 = 64; // 0.80 µs
const T1L: u16 = 36; // 0.45 µs
/// Full brightness is uncomfortably bright on the bench: 1/8.
const BRIGHTNESS_SHIFT: u32 = 3;
/// One trip round the colour wheel.
const CYCLE_MS: u64 = 4_000;
const FRAME_MS: u64 = 40;

pub fn tx_config() -> TxChannelConfig {
    TxChannelConfig::default().with_clk_divider(1).with_idle_output(true).with_idle_output_level(Level::Low)
}

/// Colour wheel: hue 0-255 to (r, g, b).
fn wheel(hue: u8) -> (u8, u8, u8) {
    let h = hue as u16 * 3;
    match h {
        0..=255 => (255 - h as u8, h as u8, 0),
        256..=511 => (0, (511 - h) as u8, (h - 256) as u8),
        _ => ((h - 512) as u8, 0, (767 - h) as u8),
    }
}

/// Show the colour for `hue`, dimmed.
async fn show(led: &mut LedChannel<'_>, hue: u8) {
    let (r, g, b) = wheel(hue);
    let grb = ((g as u32) << 16 | (r as u32) << 8 | b as u32) >> BRIGHTNESS_SHIFT & 0x1F1F1F;
    let mut data = [PulseCode::end_marker(); 25];
    for (i, code) in data.iter_mut().take(24).enumerate() {
        *code = if grb >> (23 - i) & 1 == 1 {
            PulseCode::new(Level::High, T1H, Level::Low, T1L)
        } else {
            PulseCode::new(Level::High, T0H, Level::Low, T0L)
        };
    }
    // A lost frame only delays the next colour.
    let _ = led.transmit(&data).await;
}

fn hue_at(ms: u64) -> u8 {
    (ms % CYCLE_MS * 256 / CYCLE_MS) as u8
}

/// Rainbow for `ms`, then off.
#[cfg(feature = "breadboard")]
pub async fn boot_rainbow(led: &mut LedChannel<'_>, ms: u64) {
    let start = Instant::now();
    while start.elapsed().as_millis() < ms {
        show(led, hue_at(start.elapsed().as_millis() * CYCLE_MS / ms)).await;
        Timer::after_millis(FRAME_MS).await;
    }
    // Black: (0, 0, 0) is hue-independent, so send it directly.
    let mut off = [PulseCode::new(Level::High, T0H, Level::Low, T0L); 25];
    off[24] = PulseCode::end_marker();
    let _ = led.transmit(&off).await;
}

/// Set by the motion task once it has initialised RMT (clock source and
/// interrupt handler) and claimed channel 0.
#[cfg(not(feature = "breadboard"))]
pub static RMT_READY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Rainbow for as long as the firmware runs.
#[cfg(not(feature = "breadboard"))]
#[embassy_executor::task]
pub async fn rainbow_task(pin: esp_hal::peripherals::GPIO8<'static>) {
    use esp_hal::rmt::{ChannelCreator, TxChannelCreator};
    while !RMT_READY.load(core::sync::atomic::Ordering::Acquire) {
        Timer::after_millis(10).await;
    }
    // SAFETY: the motion task's `Rmt` owns the peripheral and only ever uses
    // channel 0; nothing else touches channel 1.
    let creator = unsafe { ChannelCreator::<'static, Async, 1>::steal() };
    let Ok(mut led) = creator.configure_tx(pin, tx_config()) else {
        esp_println::println!("led: RMT channel 1 unavailable");
        return;
    };
    loop {
        show(&mut led, hue_at(Instant::now().as_millis())).await;
        Timer::after_millis(FRAME_MS).await;
    }
}
