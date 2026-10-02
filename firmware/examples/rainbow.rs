//! Board check: cycles the DevKitC-1's onboard WS2812 RGB LED (GPIO8) through
//! the rainbow and prints a heartbeat over USB. Nothing else is driven.
//!
//!   cargo run --release --example rainbow
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::Level;
use esp_hal::main;
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

// WS2812 bit timings in 12.5 ns RMT ticks (80 MHz, divider 1).
const T0H: u16 = 32; // 0.40 µs
const T0L: u16 = 68; // 0.85 µs
const T1H: u16 = 64; // 0.80 µs
const T1L: u16 = 36; // 0.45 µs
/// Full brightness is uncomfortably bright on the bench.
const BRIGHTNESS_SHIFT: u32 = 3;

/// Colour wheel: hue 0-255 to (r, g, b).
fn wheel(hue: u8) -> (u8, u8, u8) {
    let h = hue as u16 * 3;
    match h {
        0..=255 => (255 - h as u8, h as u8, 0),
        256..=511 => (0, (511 - h) as u8, (h - 256) as u8),
        _ => ((h - 512) as u8, 0, (767 - h) as u8),
    }
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    println!("rainbow: WS2812 on GPIO8");

    let rmt = Rmt::new(p.RMT, Rate::from_mhz(80)).unwrap();
    let config =
        TxChannelConfig::default().with_clk_divider(1).with_idle_output(true).with_idle_output_level(Level::Low);
    let mut channel = rmt.channel0.configure_tx(p.GPIO8, config).unwrap();
    let delay = Delay::new();

    let mut hue = 0u8;
    let mut turns = 0u32;
    loop {
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
        channel = match channel.transmit(&data).unwrap().wait() {
            Ok(ch) => ch,
            Err((e, ch)) => {
                println!("rmt error: {:?}", e);
                ch
            }
        };
        delay.delay_millis(10);

        hue = hue.wrapping_add(1);
        if hue == 0 {
            turns += 1;
            println!("rainbow: {} cycles", turns);
        }
    }
}
