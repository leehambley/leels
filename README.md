# leels: single-axis electronic gearbox

A Rust/Embassy port of the gearbox mode of
[NanoEls H5](https://github.com/kachurovskiy/nanoels), cut down to one job:
the Z lead screw follows the spindle at the selected pitch. There is no WiFi,
no other modes, no X/Y axis, no keyboard or joystick, and no GCode.

```
els-core/   hardware-independent logic, no_std, host-tested (cargo test)
firmware/   esp-hal + esp-rtos (Embassy) binary for ESP32-S3 and ESP32-C6
docs/       Nextion page layout
```

## Build and flash

```sh
cd els-core && cargo test          # logic tests on the host

cd firmware
cargo c6                           # ESP32-C6: build, flash, monitor
cargo +esp s3                      # ESP32-S3: needs the Xtensa toolchain (espup)
cargo build-c6 / cargo +esp build-s3
```

Hardware constants (encoder PPR, screw pitch, motor steps, speeds, inversion
flags) are in `firmware/src/config.rs`. Pins are at the top of `main()`:
- **S3**: NanoEls H5 pinout (encoder 13/14, STEP 35, DIR 42, ENA 41, Nextion TX 43 / RX 44).
- **C6**: placeholder pins (encoder 2/3, STEP 4, DIR 5, ENA 6, Nextion TX 22 / RX 23).

The spindle encoder has open-drain outputs, so the ESP32's internal pull-ups
(already enabled) pull A/B up to 3.3 V. No level shifting is needed on either
chip. The internal pull-ups are weak (~45 kΩ), though. At 1200 PPR and
3000 rpm the A line toggles at 60 kHz, so on a longer cable add external
1–4.7 kΩ pull-ups to 3.3 V to keep the edges sharp.

Logs go to USB-Serial-JTAG because UART0's pins drive the display.

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

`motion_task` runs on a high-priority interrupt executor every 20 µs. Each tick
it reads the PCNT spindle counter, applies queued commands, computes the target
from `target = p_ref + (spindle − s_ref) · pitch·steps / (screw·counts)` using
exact integer math, and may emit one STEP edge. The pulse is one tick long,
which gives a maximum of 25 k steps/s. Acceleration is limited, and the axis
brakes into a stop. The UI and display run on the normal thread executor and
talk to motion through a command channel and a status snapshot.

## Not carried over

Saved state (pitch, stops and position are lost on power-off), backlash
compensation, TPI entry, the max-travel E-stop, and
enable/disable per axis.
