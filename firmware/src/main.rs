//! Single-axis electronic gearbox: the Z lead screw follows the spindle at the
//! selected pitch while the virtual half-nut is engaged.
//!
//! Motion pipeline (all on a high-priority interrupt executor):
//!
//! ```text
//!  PCNT spindle count ──► predict spindle at end of next segment
//!                           │
//!        commands ────────► gearbox (engaged) or jog (disengaged) ──► target
//!                           │
//!                         StepGen: velocity/accel-limited plan for one
//!                         segment → exact STEP pulse times
//!                           │
//!                         RMT plays the segment in hardware (100 ns
//!                         resolution) while the next one is planned
//! ```
//!
//! The loop is paced by RMT: each iteration starts playing the segment
//! planned last time, plans the following one, then waits for playback to
//! finish. The UI and display run as ordinary tasks on the thread executor.
#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod config;
mod setup;
mod storage;

use core::cell::Cell;

use els_core::display::{self, Status, Text, FIELDS};
use els_core::gearbox::{Gearbox, Side};
use els_core::jog::{Bounds, Jog};
use els_core::nextion::{self, Parser};
use els_core::settings::{Origin, Settings};
use els_core::spindle::Spindle;
use els_core::stepgen::{Segment, StepGen, StepGenConfig, MAX_STEPS_PER_SEGMENT};
use els_core::ui::{Action, Command, Key, Ui};
use embassy_executor::Spawner;
use embassy_futures::select::{select3, Either3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::interrupt::Priority;
use esp_hal::pcnt::{channel, unit::Unit, Pcnt};
use esp_hal::rmt::{self, PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
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

/// STEP levels: (active, idle).
fn step_levels(s: &Settings) -> (Level, Level) {
    if s.step_active_low {
        (Level::Low, Level::High)
    } else {
        (Level::High, Level::Low)
    }
}

/// RMT items for one segment: one per step plus the trailing idle/end item.
type Codes = heapless::Vec<PulseCode, { MAX_STEPS_PER_SEGMENT + 1 }>;

struct MotionHw {
    spindle: Unit<'static, 0>,
    // The RMT channel handle isn't `Send`, so it's configured inside the task.
    rmt: esp_hal::peripherals::RMT<'static>,
    step: esp_hal::gpio::AnyPin<'static>,
    dir: Output<'static>,
    // Held so the pins stay configured.
    _enable: Output<'static>,
    _encoder_a: Input<'static>,
    _encoder_b: Input<'static>,
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

    // Heap for the WiFi stack (only used in setup mode).
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    assert_eq!(config::DEFAULTS.validate(config::TIMING), Ok(()), "config::DEFAULTS are invalid");
    let mut store = storage::Store::new(p.FLASH);
    let (settings, origin) = store.load_settings(config::DEFAULTS, config::TIMING);
    let operator_state = store.load_state(settings.max_pitch_du as i64);
    println!("settings: {:?}", origin);
    setup::CURRENT.lock(|c| c.set(Some(settings)));

    let pins = take_pins!(p);

    // Spindle encoder: count both edges of A, direction from B (2 counts per line).
    // Open-drain encoder: the internal pull-ups supply the high level.
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let encoder_a = Input::new(pins.encoder_a, pull_up);
    let encoder_b = Input::new(pins.encoder_b, pull_up);
    let pcnt = Pcnt::new(p.PCNT);
    let unit = pcnt.unit0;
    let limit = els_core::spindle::PCNT_LIMIT as i16;
    unit.set_low_limit(Some(-limit)).unwrap();
    unit.set_high_limit(Some(limit)).unwrap();
    unit.set_filter(Some(settings.encoder_filter_cycles)).unwrap();
    unit.clear();
    let ch = &unit.channel0;
    ch.set_edge_signal(encoder_a.peripheral_input());
    ch.set_ctrl_signal(encoder_b.peripheral_input());
    ch.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);
    ch.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    unit.resume();

    let enabled = if settings.invert_enable { Level::Low } else { Level::High };
    let hw = MotionHw {
        spindle: unit,
        rmt: p.RMT,
        step: pins.step,
        dir: Output::new(pins.dir, Level::Low, OutputConfig::default()),
        _enable: Output::new(pins.enable, enabled, OutputConfig::default()),
        _encoder_a: encoder_a,
        _encoder_b: encoder_b,
    };

    static MOTION_EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
    let motion_executor = MOTION_EXECUTOR.init(InterruptExecutor::new(sw.software_interrupt2));
    let motion_spawner = motion_executor.start(Priority::Priority3);
    motion_spawner.spawn(motion_task(hw, settings)).unwrap();

    let uart_config = uart::Config::default().with_baudrate(config::NEXTION_BAUD);
    let (rx, tx) =
        Uart::new(p.UART1, uart_config).unwrap().with_tx(pins.nextion_tx).with_rx(pins.nextion_rx).into_async().split();
    spawner.spawn(touch_task(rx)).unwrap();
    spawner.spawn(ui_task(tx, settings, origin, store, operator_state)).unwrap();
    spawner.spawn(setup::setup_task(spawner, p.WIFI)).unwrap();
}

/// Convert a planned segment into RMT pulse codes.
fn encode(segment: &Segment, cfg: &StepGenConfig, (on, off): (Level, Level), out: &mut Codes) {
    out.clear();
    for item in segment.items(cfg) {
        let code = if item.pulse == 0 {
            // Zero length marks the end of the transmission.
            PulseCode::new(off, item.idle, off, 0)
        } else {
            PulseCode::new(off, item.idle, on, item.pulse)
        };
        let _ = out.push(code);
    }
}

/// Spindle position predicted `ahead_us` into the future from recent samples.
struct SpindlePredictor {
    history: [(u64, i64); config::SPINDLE_VELOCITY_SEGMENTS],
    next: usize,
}

impl SpindlePredictor {
    fn predict(&mut self, now_us: u64, counts: i64, ahead_us: u64) -> i64 {
        let (then_us, then_counts) = self.history[self.next];
        self.history[self.next] = (now_us, counts);
        self.next = (self.next + 1) % self.history.len();
        let dt = now_us.saturating_sub(then_us);
        if then_us == 0 || dt == 0 {
            return counts;
        }
        counts + (counts - then_counts) * ahead_us as i64 / dt as i64
    }
}

#[embassy_executor::task]
async fn motion_task(mut hw: MotionHw, settings: Settings) {
    let levels = step_levels(&settings);
    let stepgen_cfg = settings.stepgen(config::TIMING);
    // STEP pulses come from RMT channel 0; the pin idles inactive between segments.
    let rmt = Rmt::new(hw.rmt, Rate::from_mhz(config::RMT_SOURCE_MHZ)).unwrap().into_async();
    let mut step: rmt::Channel<'static, Async, rmt::Tx> = rmt
        .channel0
        .configure_tx(
            hw.step,
            TxChannelConfig::default()
                .with_clk_divider(config::RMT_DIVIDER)
                .with_idle_output(true)
                .with_idle_output_level(levels.1),
        )
        .unwrap();

    let machine = settings.machine();
    let segment_us = config::SEGMENT_US as u64;
    let mut spindle = Spindle::new(settings.encoder_backlash as i64);
    let mut predictor = SpindlePredictor { history: [(0, 0); config::SPINDLE_VELOCITY_SEGMENTS], next: 0 };
    let mut gearbox = Gearbox::new(machine);
    let mut stepgen = StepGen::new(stepgen_cfg);
    let mut jog = Jog::new(settings.jog());
    let mut z_zero = 0i64;
    let mut turn_zero = 0i64;

    let mut rpm = 0u32;
    let mut rpm_start = (Instant::now().as_micros(), 0i64);
    let mut status_countdown = 0u32;

    // Double buffer: `playing` is on the wire while `planned` is filled.
    let mut playing = Codes::new();
    let mut planned = Codes::new();
    let mut planned_positive = true;
    encode(&Segment::default(), &stepgen_cfg, levels, &mut planned);

    loop {
        core::mem::swap(&mut playing, &mut planned);
        // The previous segment has finished, so DIR can change now; the step
        // generator keeps the first STEP edge after a change DIR_SETUP_NS away.
        hw.dir.set_level(Level::from(planned_positive != settings.invert_dir));
        // Starts the hardware immediately; awaited at the end of the loop.
        let transmission = step.transmit(&playing);

        let now = Instant::now().as_micros();
        let raw = hw.spindle.value();
        spindle.feed(if settings.invert_spindle { -raw } else { raw });
        // The segment being planned ends two segments from now.
        let s = predictor.predict(now, spindle.avg, 2 * segment_us);

        while let Ok(cmd) = COMMANDS.try_receive() {
            match cmd {
                Command::SetPitchDu(du) => gearbox.set_pitch_du(du, s),
                Command::Engage => {
                    jog.cancel();
                    gearbox.engage(s, stepgen.pos());
                    turn_zero = spindle.pos;
                }
                Command::Disengage => {
                    jog.cancel();
                    gearbox.disengage(stepgen.pos());
                }
                Command::Jog { left, distance_du } if !gearbox.engaged() => {
                    let dir = if left { 1 } else { -1 };
                    jog.press(dir, machine.du_to_steps(distance_du), stepgen.pos(), stepgen.braking_steps(), now);
                }
                Command::Jog { .. } => {}
                Command::JogRelease => jog.release(stepgen.pos(), stepgen.braking_steps(), now),
                Command::ToggleStop(side) => gearbox.toggle_stop(side, s, stepgen.pos()),
                Command::ZeroZ => z_zero = stepgen.pos(),
                Command::ZeroTurns => turn_zero = spindle.pos,
            }
        }

        let target = if gearbox.engaged() {
            stepgen.set_speed_cap(None);
            gearbox.update(s)
        } else {
            let bounds = Bounds::new(gearbox.stop(Side::Right), gearbox.stop(Side::Left));
            let (target, cap) = jog.update(stepgen.pos(), bounds, now);
            stepgen.set_speed_cap(cap);
            target
        };
        let segment = stepgen.next_segment(target);
        planned_positive = segment.positive;
        encode(&segment, &stepgen_cfg, levels, &mut planned);

        if now - rpm_start.0 >= config::RPM_WINDOW_US {
            let counts = (spindle.pos - rpm_start.1).unsigned_abs();
            let elapsed = now - rpm_start.0;
            rpm = (counts * 60_000_000 / (machine.counts_per_rev as u64 * elapsed)) as u32;
            rpm_start = (now, spindle.pos);
        }

        if status_countdown == 0 {
            status_countdown = config::STATUS_EVERY_SEGMENTS;
            let status = Status {
                engaged: gearbox.engaged(),
                syncing: gearbox.syncing(),
                jogging: jog.active(),
                pos: stepgen.pos(),
                z_zero,
                left_stop: gearbox.stop(Side::Left),
                right_stop: gearbox.stop(Side::Right),
                rpm,
                turn_counts: spindle.pos - turn_zero,
            };
            STATUS.lock(|st| st.set(status));
        }
        status_countdown -= 1;

        if let Err(e) = transmission.await {
            println!("rmt error: {:?}", e);
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
async fn ui_task(
    mut tx: UartTx<'static, Async>,
    settings: Settings,
    origin: Origin,
    mut store: storage::Store,
    operator_state: Option<els_core::settings::OperatorState>,
) {
    let machine = settings.machine();
    let mut ui = Ui::new(settings.max_pitch_du as i64);
    if let Some(state) = operator_state {
        ui.restore(state);
    }
    COMMANDS.send(Command::SetPitchDu(ui.pitch.du())).await;

    #[cfg(feature = "setup-on-boot")]
    if ui.handle(Key::Setup, 0, &STATUS.lock(|s| s.get())).action == Some(Action::StartSetup) {
        setup::START.signal(());
    }

    Timer::after_millis(config::NEXTION_BOOT_MS).await;
    let boot_ms = Instant::now().as_millis();
    match origin {
        Origin::Saved | Origin::FirstBoot => {}
        Origin::Corrupt => ui.notify("Settings damaged: defaults", boot_ms, 10_000),
        Origin::Unsupported => ui.notify("Newer settings: defaults", boot_ms, 10_000),
    }
    let mut shown: Option<[Text; FIELDS.len()]> = None;
    let mut last_full = Instant::now();
    let mut saved_state = ui.operator_state();
    let mut state_changed_at: Option<Instant> = None;
    let mut restart_at: Option<Instant> = None;

    loop {
        let now_ms = Instant::now().as_millis();
        let event =
            select3(KEYS.receive(), setup::SAVE_REQUEST.receive(), Timer::after_millis(config::DISPLAY_REFRESH_MS))
                .await;
        let status = STATUS.lock(|s| s.get());
        let idle = !status.engaged && !status.jogging;
        match event {
            Either3::First(key) => {
                let out = ui.handle(key, now_ms, &status);
                if let Some(cmd) = out.command {
                    COMMANDS.send(cmd).await;
                    // Let the motion task apply it before we render its status.
                    Timer::after_micros(2 * config::SEGMENT_US as u64).await;
                }
                if out.beep {
                    write_all(&mut tx, nextion::BEEP).await;
                }
                match out.action {
                    Some(Action::StartSetup) => setup::START.signal(()),
                    Some(Action::ExitSetup) => esp_hal::system::software_reset(),
                    None => {}
                }
            }
            Either3::Second(new_settings) => {
                // Flash writes stall the CPU, so only while nothing moves.
                let result = if idle { store.save_settings(&new_settings) } else { Err("Stop jogging first") };
                if result.is_ok() {
                    // Give the web page time to confirm, then restart with the new settings.
                    restart_at = Some(Instant::now() + Duration::from_millis(1_500));
                    ui.notify("Settings saved: restarting", now_ms, 10_000);
                }
                setup::SAVE_RESULT.signal(result);
            }
            Either3::Third(()) => {}
        }

        if restart_at.is_some_and(|t| Instant::now() >= t) {
            esp_hal::system::software_reset();
        }

        // Remember pitch, unit and step sizes once they've settled and the carriage is idle.
        let state = ui.operator_state();
        if state != saved_state {
            let changed = *state_changed_at.get_or_insert_with(Instant::now);
            if idle && changed.elapsed() >= Duration::from_millis(config::STATE_SAVE_DELAY_MS) {
                if let Err(e) = store.save_state(&state) {
                    println!("state save failed: {}", e);
                }
                saved_state = state;
                state_changed_at = None;
            }
        }

        if last_full.elapsed() >= Duration::from_millis(config::DISPLAY_FULL_REFRESH_MS) {
            shown = None;
            last_full = Instant::now();
        }
        let status = STATUS.lock(|s| s.get());
        let fields = display::render(&ui, &status, &machine, Instant::now().as_millis(), config::SETUP_BANNER);
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
