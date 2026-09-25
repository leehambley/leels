# Nextion page layout

Only page 0 is used. The firmware identifies buttons by **component id**
(Nextion sends `65 00 <id> 01 FF FF FF` on press when *Send Component ID* is
ticked for the Touch Press Event). It writes text fields by **objname**.

Ids below 51 are the existing NanoEls H5 `h5.hmi` ids, so you can edit that
file: delete what you don't need (mode page, X/Y axis, joystick…), keep these
ids, and add the new buttons. The mapping lives in
`els-core/src/nextion.rs::key_for`.

## Buttons (touch press → action)

| id | suggested objname | action |
|----|-------------------|--------|
| 25 | bOn | **Engage** virtual half-nut (refused while a number is being typed) |
| 23 | bOff | **Disengage** |
| 3 | bStatus | Disengage (tap status) |
| 5 | bReverse | **Rev**: flip pitch direction |
| 6 | bMeasure | Toggle **MM / IN** (number kept; refused if it would exceed 1") |
| 42 | bPlus | Pitch magnitude + selected step |
| 43 | bMinus | Pitch magnitude − selected step |
| 53 | bStep001 | Step .001 (new) |
| 54 | bStep01 | Step .01 (new) |
| 55 | bStep1 | Step .1 (new) |
| 56 | bStep10 | Step 1.0 (new) |
| 57 | bStep100 | Step 10.0 (new) |
| 7 | tStepVal | Cycle step size |
| 26–35 | b0…b9 | Digits 0–9 |
| 52 | bPoint | Decimal point (new) |
| 24 | bBack | Backspace |
| 51 | bEnter | Apply typed pitch (new) |
| 40 | bStopL | Set / clear Z left stop at current position |
| 41 | bStopR | Set / clear Z right stop at current position |
| 21 | bZ0 | Zero the Z position readout |
| 48 | bLeft | **Jog left**, needs press *and* release events (see below) |
| 49 | bRight | **Jog right**, needs press *and* release events |
| 58 | bJogDist | Cycle jog distance .01 / .1 / 1 / 10 (new) |
| 59 | bSetup | Start / leave the WiFi setup hotspot (new) |
| 9, 10 | tTurns, tAngle | Zero turns and angle readouts |

For the two jog buttons, tick *Send Component ID* on **both** Touch Press and
Touch Release events. Every other button only needs the press event.

## Text fields written by the firmware

| objname | content |
|---------|---------|
| bStatus | `OFF`, `ON` (engaged), `SYN` (waiting for thread phase) or `SET` (setup hotspot on) |
| tPitch | signed pitch, 3 decimals, e.g. `-1.250` |
| bMeasure | `MM` / `IN` |
| tStepVal | selected step: `0.001` … `10` |
| tRPMVal | spindle RPM |
| tTurnsVal | spindle turns since engage (or last zero) |
| tAngleVal | spindle angle since engage, degree sign sent as byte 0xDF |
| tZ | Z position (mm 3 dp / inch 4 dp) |
| tZLeft / tZRight | distance to left / right stop, blank when unset |
| tJogVal | jog distance: `0.01` … `10` (new) |
| t3 | typed number (`Pitch 1.25`), warnings, `Set pitch`, hotspot name and password in setup mode |

Fields are only resent when their text changes, plus a full refresh every 5 s.
