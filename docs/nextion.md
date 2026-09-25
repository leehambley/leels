# Nextion page layout

Only page 0 of the Lee-LS HMI (`nextion/lee-ls.HMI`, compiled as
`nextion/lee-ls.tft`) is used. The firmware identifies buttons by
**component id**. The ids live in `els-core/src/nextion.rs::id`; the Nextion
Editor renumbers every component after a deleted one, so re-check them
whenever the HMI changes. Nextion sends `65 00 <id> 01 FF FF FF` on press when *Send
Component ID* is ticked for the Touch Press Event. It writes text by
**objname**. The mapping lives in `els-core/src/nextion.rs::key_for`; the
written fields are in `els-core/src/display.rs::FIELDS`.

See `nextion-hmi-lee-ls.md` for the full component list with geometry,
and the history of the naming decisions.

## Buttons (touch press → action)

| id | objname | action |
|---:|---------|--------|
| 19 | bToggleEngaged | **Engage** the virtual half-nut when idle (refused while a number is being typed, in setup, or while jogging); **disengage** when engaged or syncing |
| 20 | bReverseToggle | **Reverse**: flip pitch direction |
| 21 | bUnitsToggle | Toggle **MM / IN** (number kept; refused if it would exceed 1") |
| 24 | bPitch001 | Set pitch to 0.01 (current unit and direction), a keypad shortcut |
| 25 | bPitch01 | Set pitch to 0.1 |
| 26 | bPitch1 | Set pitch to 1.0 |
| 6–8, 10–16 | bNum1…bNum9, bNum0 | Digits (`bNum1`=6, `bNum2`=7, `bNum3`=8, `bNum4`…`bNum9`=10…15, `bNum0`=16) |
| 18 | bNumPeriod | Decimal point |
| 9 | bBackspace | Delete the last typed character; **hold ≥ 0.7 s** (on release) to clear the whole entry. Needs press *and* release events |
| 17 | bNumOK | Apply typed pitch |
| 4 | bLeftStop | Set the Z left stop at the current position, or clear it (see *Stops* below) |
| 5 | bRightStop | Same for the right stop |
| 27 | bZeroZ | Zero the Z position readout **and clear both stops** (`Stops cleared` on the message line for 5 s or until the next press). Refused while engaged with a stop set |
| 28 | bJogL | **Jog left**; needs press *and* release events |
| 29 | bJogR | **Jog right**; needs press *and* release events |
| 30 | bCycleJogDist | Cycle jog distance .01 / .1 / 1 / 10 (current unit) |
| 32 | bSettings | Start / leave the WiFi setup hotspot |
| 2, 3 | tAngle, tTurns | Zero turns and angle readouts |

For the two jog buttons and `bBackspace`, tick *Send Component ID* on **both**
Touch Press and Touch Release events. Every other entry above only needs the
press event.

**Pitch is locked while the half-nut is engaged:** digits, `.`, OK, the pitch
presets, REVERSE and mm/inch beep and show `Disengage to change pitch`.

**Keypad entry:** while a number is typed but not yet applied, `tPitch` shows
it with a blinking cursor (`PITCH 1.25_`) on an amber background, and the
message line shows `OK to apply, <- delete, hold <- clear`. Any other button
drops the entry and shows `Entry cancelled`; a pitch preset simply replaces it.

## Text written by the firmware

| objname | content |
|---------|---------|
| bToggleEngaged | `Engage`, `Disengage` (engaged), `Syncing` (waiting for thread phase) or `Setup on` |
| tPitch | `PITCH -1.250mm`; while typing `PITCH 1.25_` (blinking cursor, amber) |
| bReverseToggle | `NORMAL` (grey) / `REVERSE` (amber) |
| tMessageLine | (max 40 chars) the typed number (`Pitch 1.25`), warnings, `Set pitch`, `Waiting for thread phase`, and the hotspot name and password in setup mode |
| bUnitsToggle | `UNIT: MM` / `UNIT: IN` (max 10 chars) |
| tRPM | `RPM 1200` |
| tTurns | `TURNS 12.34`, spindle turns since engage (or last zero) |
| tAngle | `ANGLE 123.45°`, degree sign sent as byte 0xDF |
| tZPos | `Z POS 1.123mm` (mm 3 dp / inch 4 dp) |
| bLeftStop | `\|< Set stop`, `\|< 12.345mm` (the stop's Z position), or `\|< AT STOP`; max 15 chars |
| bRightStop | `Set stop >\|`, `12.345mm >\|`, or `AT STOP >\|` |
| bCycleJogDist | idle: `JOG DIST 0.1mm`; tap move: `JOG 0.300mm` (distance still to go); hold: `HOLD SPEED 2/4`; braking: `STOPPING` |

Fields are only resent when their
text changes, plus a full refresh every 5 s.

## Stops

Each stop button has three states. The firmware sets the caption and the
background (`bco`) and text (`pco`) colours. Every pair meets WCAG AAA
(≥ 7:1 contrast) for poor workshop lighting; a unit test enforces it:

| State | Caption | `bco` | Tap |
|---|---|---|---|
| Not set | `\|< Set stop` | grey `50712` / black text (11.9:1) | set the stop at the current position |
| Set | `\|< 12.345mm` = the stop's Z position (same zero and units as `tZPos`) | amber `64896` / black text (11.6:1) | clear it, **only while disengaged**; otherwise beep + `Stop armed: wait or disengage` |
| Parked (carriage is at the stop) | `\|< AT STOP` | dark red `40960` / white text (8.1:1) | clear it: when threading, the carriage waits for thread phase and continues |

"Parked" means the motor position equals the stop: either a threading pass
ended there, or a jog ran into it.

Messages on `tMessageLine` disappear after their timeout or on the next
button press (jog releases don't count).
