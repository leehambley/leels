# leels: single-axis electronic gearbox

A Rust/Embassy port of the gearbox mode of
[NanoEls H5](https://github.com/kachurovskiy/nanoels), cut down to one job:
the Z lead screw follows the spindle at the selected pitch. There is no WiFi,
no other modes, no X/Y axis, no keyboard or joystick, and no GCode.

```
els-core/   hardware-independent logic, no_std, host-tested (cargo test)
firmware/   esp-hal + esp-rtos (Embassy) binary; ESP32-C6 primary, ESP32-S3 secondary
docs/       Nextion page layout
```

## Build and flash

```sh
cd els-core && cargo test          # logic tests on the host

cd firmware
cargo run --release                # ESP32-C6 (default, stable Rust): build, flash, monitor
cargo build --release
cargo +esp s3                      # ESP32-S3: needs the Xtensa toolchain (espup)
```

Everything machine- or board-specific is in `firmware/src/config.rs`, grouped
into pinout, spindle encoder, lead screw and stepper, motion timing, jogging
and operator interface. Impossible combinations fail the build: a pulse too
wide for the maximum speed, or a segment too long.
- **C6** pins (placeholders; change for your board): encoder 2/3, STEP 4, DIR 5, ENA 6, Nextion TX 22 / RX 23.
- **S3** pins: NanoEls H5 pinout (encoder 13/14, STEP 35, DIR 42, ENA 41, Nextion TX 43 / RX 44).

The spindle encoder has open-drain outputs, so the ESP32's internal pull-ups
(already enabled) pull A/B up to 3.3 V. No level shifting is needed on either
chip. The internal pull-ups are weak (~45 kΩ), though. At 1200 PPR and
3000 rpm the A line toggles at 60 kHz, so on a longer cable add external
1–4.7 kΩ pull-ups to 3.3 V to keep the edges sharp.

Logs go to USB-Serial-JTAG, which leaves both UARTs free.

## Operation

- **Pitch** is a number plus a unit. Choose a step size (.001 / .01 / .1 / 1 / 10)
  and use **+/−**, or type a number with a decimal point and press **Enter**.
  **Rev** flips direction. **MM/IN** keeps the number and changes the unit
  (0.100 mm becomes 0.100"). Pitch is capped at 1".
- **Engage** closes the virtual half-nut at the current position.
  **Disengage** stops the lead screw immediately. Changing the pitch while
  engaged continues smoothly from the current position.
- **Stops**: pressing Stop L or Stop R sets that stop at the current position,
  and pressing it again clears it. Stops apply only while engaged, because the
  axis doesn't move otherwise. The carriage parks on a stop while the spindle
  keeps turning. Reverse the spindle and it leaves the stop within one turn,
  still in thread phase. If you clear a stop while resting on it, the screen
  shows `SYN` and the carriage waits for the thread phase before moving on.
- **Jog** (disengaged only; refused with a beep while engaged): **Jog L / Jog R**
  move the carriage. Stops limit jogging too.
  - A **tap** moves exactly the jog distance (.01 / .1 / 1 / 10 in the current
    unit, cycled with its own button). Repeated taps add up.
  - A **hold** longer than 0.4 s runs continuously. It starts at 0.5 mm/s and
    steps up to 2, then 8, then 25 mm/s every second. Releasing brakes to a
    stop. Pressing the other direction while moving also brakes.
  - Speeds and timings are in `firmware/src/config.rs` (`JOG_*`).

## How motion works

Software plans the motion and hardware times the pulses: the same split as
LinuxCNC's stepgen or Klipper.

```
 PCNT spindle count ─► predict spindle position at end of next segment
                         │
       commands ───────► gearbox (engaged) or jog (disengaged) ─► target
                         │
                       StepGen: velocity/acceleration-limited plan for one
                       250 µs segment → exact STEP pulse times
                         │
                       RMT plays the segment in hardware (100 ns resolution)
                       while the next segment is planned
```

- `motion_task` runs on a high-priority interrupt executor, paced by the RMT
  peripheral. Each loop starts playing the segment planned last time, plans
  the next one, then waits for playback to finish. That's 4,000 wakeups a second.
- The gearbox target is `p_ref + (spindle − s_ref) · pitch·steps / (screw·counts)`,
  in exact integer maths, including the fraction of a step. It's evaluated at
  the spindle position predicted for the end of the segment, so the carriage
  doesn't lag the spindle.
- StepGen (`els-core/src/stepgen.rs`) sets each segment's velocity from:
  - feed-forward: how fast the target is moving;
  - a correction for any remaining error, limited so the error could still be
    braked away (`√(2·a·error)`).

  Velocity is constant within a segment, so pulses are evenly spaced. Host tests
  check a steady spindle gives step intervals within ±0.1 µs of ideal, and that
  a realistic spindle run-up never lags by more than 1 step.
- The maximum step rate is set by the pulse width and the driver (30 k steps/s
  configured), not by a software tick.
- The UI and display run on the normal thread executor. They talk to motion
  through a command channel and a status snapshot.

Known limitation: esp-hal's RMT driver restarts transmission for each
segment, which leaves a few-µs gap between segments. Position isn't affected,
because every step is still planned against the spindle. A gapless version
would need a custom RMT ring-buffer driver.

## Not carried over

Saved state (pitch, stops and position are lost on power-off), backlash
compensation, TPI entry, the max-travel E-stop, and
enable/disable per axis.
