//! Single-axis electronic gearbox: the Z lead screw follows the spindle at the
//! selected pitch while the virtual half-nut is engaged.
//!
//! Tasks:
//! - `motion_task` (high-priority interrupt executor, every MOTION_TICK_US):
//!   spindle counter → gearbox → step/dir pins. Owns all motion state.
//! - `touch_task`: parses Nextion touch events into keys.
//! - `ui_task`: applies keys, sends commands to motion, redraws the display.
#![no_std]
#![no_main]

mod config;

use core::cell::Cell;

use els_core::display::{self, Status, Text, FIELDS};
use els_core::gearbox::{Gearbox, Side};
use els_core::jog::{Bounds, Jog};
use els_core::nextion::{self, Parser};
use els_core::spindle::Spindle;
use els_core::stepper::{Action, Stepper};
use els_core::ui::{Command, Key, Ui};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::interrupt::Priority;
use esp_hal::pcnt::{channel, unit::Unit, Pcnt};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{self, Uart, UartRx, UartTx};
use esp_hal::Async;
use esp_println::println;
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

static COMMANDS: Channel<CriticalSectionRawMutex, Command, 16> = Channel::new();
static KEYS: Channel<CriticalSectionRawMutex, Key, 16> = Channel::new();
static STATUS: Mutex<CriticalSectionRawMutex, Cell<Status>> = Mutex::new(Cell::new(Status {
    engaged: false,
    syncing: false,
    jogging: false,
    pos: 0,
    z_zero: 0,
    left_stop: None,
    right_stop: None,
    rpm: 0,
    turn_counts: 0,
}));

struct MotionHw {
    spindle: Unit<'static, 0>,
    step: Output<'static>,
    dir: Output<'static>,
    // Held so the pins stay configured.
    _enable: Output<'static>,
    _enc_a: Input<'static>,
    _enc_b: Input<'static>,
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timg0 = TimerGroup::new(p.TIMG0);
    let sw = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    #[cfg(feature = "esp32c6")]
    esp_rtos::start(timg0.timer0, sw.software_interrupt0);
    #[cfg(feature = "esp32s3")]
    esp_rtos::start(timg0.timer0);
    println!("ELS gearbox starting");

    // NanoEls H5 pinout.
    #[cfg(feature = "esp32s3")]
    let (enc_a, enc_b, step, dir, enable, nx_tx, nx_rx) =
        (p.GPIO13, p.GPIO14, p.GPIO35, p.GPIO42, p.GPIO41, p.GPIO43, p.GPIO44);
    // ESP32-C6: no reference board yet, adjust to your wiring (see README).
    #[cfg(feature = "esp32c6")]
    let (enc_a, enc_b, step, dir, enable, nx_tx, nx_rx) =
        (p.GPIO2, p.GPIO3, p.GPIO4, p.GPIO5, p.GPIO6, p.GPIO22, p.GPIO23);

    // Spindle encoder: count both edges of A, direction from B (2 counts per line).
    // Open-drain encoder: the internal pull-ups supply the high level.
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let enc_a = Input::new(enc_a, pull_up);
    let enc_b = Input::new(enc_b, pull_up);
    let pcnt = Pcnt::new(p.PCNT);
    let unit = pcnt.unit0;
    let limit = els_core::spindle::PCNT_LIMIT as i16;
    unit.set_low_limit(Some(-limit)).unwrap();
    unit.set_high_limit(Some(limit)).unwrap();
    unit.set_filter(Some(config::ENCODER_FILTER_CYCLES)).unwrap();
    unit.clear();
    let ch = &unit.channel0;
    ch.set_edge_signal(enc_a.peripheral_input());
    ch.set_ctrl_signal(enc_b.peripheral_input());
    ch.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);
    ch.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    unit.resume();

    let step_idle = if config::STEP_ACTIVE_LOW { Level::High } else { Level::Low };
    let enabled = if config::INVERT_ENABLE { Level::Low } else { Level::High };
    let hw = MotionHw {
        spindle: unit,
        step: Output::new(step, step_idle, OutputConfig::default()),
        dir: Output::new(dir, Level::Low, OutputConfig::default()),
        _enable: Output::new(enable, enabled, OutputConfig::default()),
        _enc_a: enc_a,
        _enc_b: enc_b,
    };

    static MOTION_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
    let motion_executor = MOTION_EXECUTOR.init(InterruptExecutor::new(sw.software_interrupt2));
    let motion_spawner = motion_executor.start(Priority::Priority3);
    motion_spawner.spawn(motion_task(hw)).unwrap();

    let uart_config = uart::Config::default().with_baudrate(config::NEXTION_BAUD);
    let (rx, tx) = Uart::new(p.UART1, uart_config)
        .unwrap()
        .with_tx(nx_tx)
        .with_rx(nx_rx)
        .into_async()
        .split();
    spawner.spawn(touch_task(rx)).unwrap();
    spawner.spawn(ui_task(tx)).unwrap();
}

#[embassy_executor::task]
async fn motion_task(mut hw: MotionHw) {
    let machine = config::machine();
    let mut spindle = Spindle::new(config::ENCODER_BACKLASH);
    let mut gearbox = Gearbox::new(machine);
    let mut stepper = Stepper::new(config::stepper());
    let mut jog = Jog::new(config::jog());
    let mut z_zero = 0i64;
    let mut turn_zero = 0i64;
    let mut pulse_active = false;
    let (step_on, step_off) = if config::STEP_ACTIVE_LOW { (Level::Low, Level::High) } else { (Level::High, Level::Low) };

    let mut rpm = 0u32;
    let mut rpm_start = (Instant::now().as_micros(), 0i64);
    let mut next_status = 0u64;

    let mut ticker = Ticker::every(Duration::from_micros(config::MOTION_TICK_US));
    loop {
        ticker.next().await;
        let now = Instant::now().as_micros();

        let raw = hw.spindle.value();
        spindle.feed(if config::INVERT_SPINDLE { -raw } else { raw });

        while let Ok(cmd) = COMMANDS.try_receive() {
            match cmd {
                Command::SetPitchDu(du) => gearbox.set_pitch_du(du, spindle.avg),
                Command::Engage => {
                    jog.cancel();
                    gearbox.engage(spindle.avg, stepper.pos);
                    turn_zero = spindle.pos;
                }
                Command::Disengage => {
                    jog.cancel();
                    gearbox.disengage(stepper.pos);
                }
                Command::Jog { left, distance_du } if !gearbox.engaged() => {
                    let dir = if left { 1 } else { -1 };
                    jog.press(dir, machine.du_to_steps(distance_du), stepper.pos, stepper.braking_steps(), now);
                }
                Command::Jog { .. } => {}
                Command::JogRelease => jog.release(stepper.pos, stepper.braking_steps(), now),
                Command::ToggleStop(side) => gearbox.toggle_stop(side, spindle.avg, stepper.pos),
                Command::ZeroZ => z_zero = stepper.pos,
                Command::ZeroTurns => turn_zero = spindle.pos,
            }
        }

        let target = if gearbox.engaged() {
            stepper.set_speed_cap(None);
            gearbox.update(spindle.avg)
        } else {
            let bounds = Bounds::new(gearbox.stop(Side::Right), gearbox.stop(Side::Left));
            let (target, cap) = jog.update(stepper.pos, bounds, now);
            stepper.set_speed_cap(cap);
            target
        };
        if pulse_active {
            // End the pulse started last tick; no new edge this tick.
            hw.step.set_level(step_off);
            pulse_active = false;
        } else {
            match stepper.poll(now, target) {
                Action::Idle => {}
                Action::SetDirection(positive) => {
                    hw.dir.set_level(Level::from(positive != config::INVERT_DIR));
                }
                Action::Step => {
                    hw.step.set_level(step_on);
                    pulse_active = true;
                }
            }
        }

        if now - rpm_start.0 >= config::RPM_WINDOW_US {
            let counts = (spindle.pos - rpm_start.1).unsigned_abs();
            let elapsed = now - rpm_start.0;
            rpm = (counts * 60_000_000 / (machine.counts_per_rev as u64 * elapsed)) as u32;
            rpm_start = (now, spindle.pos);
        }

        if now >= next_status {
            next_status = now + config::STATUS_PERIOD_US;
            let status = Status {
                engaged: gearbox.engaged(),
                syncing: gearbox.syncing(),
                jogging: jog.active(),
                pos: stepper.pos,
                z_zero,
                left_stop: gearbox.stop(Side::Left),
                right_stop: gearbox.stop(Side::Right),
                rpm,
                turn_counts: spindle.pos - turn_zero,
            };
            STATUS.lock(|s| s.set(status));
        }
    }
}

#[embassy_executor::task]
async fn touch_task(mut rx: UartRx<'static, Async>) {
    let mut parser = Parser::default();
    let mut buf = [0u8; 32];
    loop {
        let n = match rx.read_async(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                println!("nextion rx error: {:?}", e);
                continue;
            }
        };
        for &b in &buf[..n] {
            if let Some(key) = parser.push(b).and_then(nextion::key_for) {
                KEYS.send(key).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn ui_task(mut tx: UartTx<'static, Async>) {
    let machine = config::machine();
    let mut ui = Ui::new(config::MAX_PITCH_DU);
    COMMANDS.send(Command::SetPitchDu(ui.pitch.du())).await;

    Timer::after_millis(config::NEXTION_BOOT_MS).await;
    let mut shown: Option<[Text; FIELDS.len()]> = None;
    let mut last_full = Instant::now();

    loop {
        let now_ms = Instant::now().as_millis();
        if let Either::First(key) = select(KEYS.receive(), Timer::after_millis(config::DISPLAY_REFRESH_MS)).await {
            let out = ui.handle(key, now_ms, &STATUS.lock(|s| s.get()));
            if let Some(cmd) = out.command {
                COMMANDS.send(cmd).await;
                // Let the motion task apply it before we render its status.
                Timer::after_micros(2 * config::MOTION_TICK_US).await;
            }
            if out.beep {
                write_all(&mut tx, nextion::BEEP).await;
            }
        }

        if last_full.elapsed() >= Duration::from_millis(config::DISPLAY_FULL_REFRESH_MS) {
            shown = None;
            last_full = Instant::now();
        }
        let status = STATUS.lock(|s| s.get());
        let fields = display::render(&ui, &status, &machine, Instant::now().as_millis());
        for (i, text) in fields.iter().enumerate() {
            if shown.as_ref().is_some_and(|s| s[i] == *text) {
                continue;
            }
            let mut cmd: heapless::Vec<u8, 64> = heapless::Vec::new();
            if nextion::encode_text(&mut cmd, FIELDS[i], text).is_ok() {
                write_all(&mut tx, &cmd).await;
            }
        }
        shown = Some(fields);
    }
}

async fn write_all(tx: &mut UartTx<'static, Async>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        match tx.write_async(bytes).await {
            Ok(n) => bytes = &bytes[n..],
            Err(e) => {
                println!("nextion tx error: {:?}", e);
                return;
            }
        }
    }
}
