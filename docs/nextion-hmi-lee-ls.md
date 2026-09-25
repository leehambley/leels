# Lee-LS HMI → firmware mapping

> **Historical.** This compared an earlier revision of the HMI with the
> firmware before it was adapted. The committed HMI (`nextion/lee-ls.HMI`)
> has a different layout, no `tSpeed`, an added `tMessageLine`, and ids shifted
> by one. `docs/nextion.md` is the current reference.

This compares `Lee-LS HMI.grid.HMI` (page 0, 33 components) with the
current firmware: `els-core/src/nextion.rs::key_for`, the `Key` enum in
`els-core/src/ui.rs`, `els-core/src/display.rs::FIELDS`, and
`docs/nextion.md`.

**Status:** §2, §3 and §5 are implemented in `els-core`, using the
decisions: bPitch buttons are pitch presets (`Key::PitchPreset`), messages
go to the added `tMessageLine` (40 chars), `tSpeed` is dropped (remove it from the HMI).

Component ids come from the HMI's `id` attribute (hex in the file,
decimal below). The grid version has the same ids as the original.

> **Check in the Editor:** the "Send Component ID" flags on the Touch
> Press/Release events were not decoded from the HMI. Every button below
> needs *Press* ticked. `bJogL`/`bJogR` also need *Release* ticked, and so
> do `tTurns`/`tAngle` if they stay tappable.

## 1. Component list

| id | objname | type | label in HMI | font | geometry (x,y w×h) |
|---:|---|:-:|---|:-:|---|
| 1 | `tRPM` | t | RPM | 0 | 14,6 148×30 |
| 2 | `tSpeed` | t | SPEED | 0 | 170,6 148×30 (dropped, to be removed) |
| 3 | `tAngle` | t | ANGLE | 0 | 326,6 148×30 |
| 4 | `tTurns` | t | TURNS | 0 | 482,6 148×30 |
| 5 | `bLeftStop` | b | \|< LStop | 0 | 482,396 148×70 |
| 6 | `bRightStop` | b | RStop >\| | 0 | 638,396 148×70 |
| 7 | `bNum1` | b | 1 | 0 | 482,84 70×70 |
| 8 | `bNum2` | b | 2 | 0 | 560,84 70×70 |
| 9 | `bNum3` | b | 3 | 0 | 638,84 70×70 |
| 10 | `bBackspace` | b | <- | 0 | 716,84 70×70 |
| 11 | `bNum4` | b | 4 | 0 | 482,162 70×70 |
| 12 | `bNum5` | b | 5 | 0 | 560,162 70×70 |
| 13 | `bNum6` | b | 6 | 0 | 638,162 70×70 |
| 14 | `bNum7` | b | 7 | 0 | 482,240 70×70 |
| 15 | `bNum8` | b | 8 | 0 | 560,240 70×70 |
| 16 | `bNum9` | b | 9 | 0 | 638,240 70×70 |
| 17 | `bNum0` | b | 0 | 0 | 560,318 70×70 |
| 18 | `bNumOK` | b | OK | 0 | 716,162 70×226 |
| 19 | `bNumPeriod` | b | . | 0 | 482,318 70×70 |
| 20 | `bToggleEngaged` | b | Engage | 0 | 14,84 226×148 |
| 21 | `bReverseToggle` | b | REVERSE | 0 | 14,240 109×70 |
| 22 | `bUnitsToggle` | b | mm/inch | 0 | 131,240 109×70 |
| 23 | `bStepCycle` | b | STEP CYCLE | 0 | 14,318 226×70 |
| 24 | `tPitch` | t | PITCH 1mm | 1 | 248,84 226×70 |
| 25 | `bPitch001` | b | 0.01 | 0 | 14,396 70×70 |
| 26 | `bPitch01` | b | 0.1 | 0 | 92,396 70×70 |
| 27 | `bPitch1` | b | 1.0 | 0 | 170,396 70×70 |
| 28 | `bZeroZ` | b | Zero Z | 0 | 248,240 226×70 |
| 29 | `bJogL` | b | JOG < | 0 | 248,318 109×70 |
| 30 | `bJogR` | b | > Jog | 0 | 365,318 109×70 |
| 31 | `bCycleJogDist` | b | JOG DIST 0.1mm | 0 | 248,396 226×70 |
| 32 | `tZPos` | t | Z POS 1.123mm | 1 | 248,162 226×70 |
| 33 | `bSettings` | b | CFG | 0 | 716,6 70×70 |

## 2. Touch → `Key` mapping

**Status** key:
- ✅ same action; only the id or name changed
- 🆕 needs a new `Key` variant or new logic
- ❓ needs your decision

| id | objname | → Key | old firmware id / name | status |
|---:|---|---|---|:-:|
| 7–9, 11–16, 17 | `bNum1`…`bNum9`, `bNum0` | `Digit(n)` | 26–35, `b0`…`b9` | ✅ ids are **no longer contiguous**, so `c @ 26..=35` must become an explicit table (§5) |
| 19 | `bNumPeriod` | `Point` | 52 `bPoint` | ✅ |
| 10 | `bBackspace` | `Backspace` | 24 `bBack` | ✅ |
| 18 | `bNumOK` | `Enter` | 51 `bEnter` | ✅ |
| 20 | `bToggleEngaged` | **`ToggleEngage`** | 25 `bOn` / 23 `bOff` / 3 `bStatus` | 🆕 one button replaces three: engage when off, disengage when on or syncing. Keep the "refused while typing" rule for the engage direction. |
| 21 | `bReverseToggle` | `Reverse` | 5 `bReverse` | ✅ |
| 22 | `bUnitsToggle` | `ToggleUnit` | 6 `bMeasure` | ✅ |
| 23 | `bStepCycle` | `CycleStep` | 7 `tStepVal` | ✅ |
| 25 | `bPitch001` (0.01) | `StepSize(1)` | 54 `bStep01` | ❓ see §4.1 |
| 26 | `bPitch01` (0.1) | `StepSize(2)` | 55 `bStep1` | ❓ see §4.1 |
| 27 | `bPitch1` (1.0) | `StepSize(3)` | 56 `bStep10` | ❓ see §4.1 |
| 5 | `bLeftStop` | `StopLeft` | 40 `bStopL` | ✅ |
| 6 | `bRightStop` | `StopRight` | 41 `bStopR` | ✅ |
| 28 | `bZeroZ` | `ZeroZ` | 21 `bZ0` | ✅ |
| 29 | `bJogL` | `Jog { left: true, .. }` | 48 `bLeft` | ✅ press **and** release |
| 30 | `bJogR` | `Jog { left: false, .. }` | 49 `bRight` | ✅ press **and** release |
| 31 | `bCycleJogDist` | `JogCycle` | 58 `bJogDist` | ✅ |
| 33 | `bSettings` | `Setup` | 59 `bSetup` | ✅ |
| 3, 4 | `tAngle`, `tTurns` | `ZeroTurns` | 10, 9 (same names) | ✅ ids changed (10→3, 9→4) |
| 1, 2, 24, 32 | `tRPM`, `tSpeed`, `tPitch`, `tZPos` | none | none | not touch targets |

**Firmware keys with no HMI button:**

| Key | old id / name | note |
|---|---|---|
| `Plus` | 42 `bPlus` | ❓ no +/- buttons, see §4.1 |
| `Minus` | 43 `bMinus` | ❓ |
| `StepSize(0)` (0.001) | 53 `bStep001` | only reachable through `bStepCycle` |
| `StepSize(4)` (10) | 57 `bStep100` | only reachable through `bStepCycle` |
| `Engage` / `Disengage` | 25 / 23 | replaced by `ToggleEngage`. Keep them internally if the web UI or other code uses them. |

## 3. Text written by the firmware (`display.rs::FIELDS`)

| Firmware field | Content today | New target | status |
|---|---|---|:-:|
| `bStatus` | `OFF`/`ON`/`SYN`/`SET` | `bToggleEngaged.txt` | 🆕 suggest showing the action or state on the big button, e.g. `ENGAGE` when off, `ON` / `SYNC…` / `SETUP` otherwise |
| `tPitch` | `-1.250` | `tPitch.txt` | ✅ name matches. The HMI placeholder is `PITCH 1mm`, so the firmware probably needs to prefix/suffix it (`PITCH -1.250mm`). |
| `bMeasure` | `MM`/`IN` | `bUnitsToggle.txt` | ✅ rename |
| `tStepVal` | `0.001`…`10` | `bStepCycle.txt`? | ❓ no dedicated field. Suggest `STEP 0.1` on the cycle button. |
| `tRPMVal` | rpm | `tRPM.txt` | 🆕 one field holds label and value, e.g. `RPM 1200` |
| `tTurnsVal` | turns | `tTurns.txt` | 🆕 e.g. `TURNS 12.34` |
| `tAngleVal` | angle ° | `tAngle.txt` | 🆕 e.g. `ANGLE 123.45°` |
| none | none | `tSpeed.txt` | dropped: not a firmware value (the H5 had no speed field either) |
| `tZ` | Z position | `tZPos.txt` | ✅ rename. Placeholder `Z POS 1.123mm` suggests the same prefix/suffix treatment as `tPitch`. |
| `tZLeft` | distance to left stop | `bLeftStop.txt`? | ❓ no field. Could show `\|< 12.345` once set and `\|< LStop` when unset. |
| `tZRight` | distance to right stop | `bRightStop.txt`? | ❓ as above |
| `tJogVal` | `0.01`…`10` | `bCycleJogDist.txt` | 🆕 placeholder is `JOG DIST 0.1mm` |
| `t3` | typed number, warnings, `Set pitch`, hotspot SSID/password | **none** | ❓ biggest gap, see §4.2 |

## 4. Open decisions

### 4.1 Pitch step buttons without +/-

`StepSize` only affects `Plus`/`Minus`, and the HMI has no +/- buttons.
The buttons labelled `0.01`, `0.1`, and `1.0` therefore do nothing
visible under the current firmware. Options:

- **a)** Add `bPitchPlus`/`bPitchMinus` to the HMI. There is no free
  cell in the grid, so something would have to shrink.
- **b)** Make them *pitch presets*: tapping `1.0` sets pitch to 1.0 mm.
  This needs a new `Key::PitchPreset(milli)`.
- **c)** Make them the **jog distance** selector instead, and drop
  `bCycleJogDist`. The values match `JOG_SIZES_MILLI` (0.01/0.1/1/10
  minus 10).
- **d)** Leave them as step-size selectors for a future +/- (knob or
  encoder).

`bStepCycle` has the same issue: it cycles a step size that nothing uses
until +/- exists.

### 4.2 No message line (`t3`)

`t3` carries:
- the number being typed (`Pitch 1.25`), the only feedback while using
  the keypad;
- warnings ("refused while typing", unit overflow);
- `Set pitch`;
- `Waiting for thread phase`;
- the setup hotspot name and password.

Options:

- **a)** While typing, and for timed messages, show them in `tPitch`
  (it is the biggest text field and sits next to the keypad), then
  revert to the pitch. Show setup credentials there too while
  `setup_active()`. This needs firmware only.
- **b)** Add a message text field to the HMI, e.g. a 1-row strip. The
  grid has no free row, so something has to shrink.

(a) seems the better fit for this layout.

### 4.3 Status display

With `bStatus` gone, the toggle button's own text is the only engaged
indicator. The HMI could also change its `bco` when engaged: the
firmware would send `bToggleEngaged.bco=<rgb565>`. `encode_text` only
does `.txt`, so a small `encode_num` would be needed.

## 5. Suggested `key_for`

```rust
pub fn key_for(touch: Touch) -> Option<Key> {
    if touch.page != 0 {
        return None;
    }
    match touch.component {
        29 => return Some(Key::Jog { left: true, pressed: touch.pressed }),  // bJogL
        30 => return Some(Key::Jog { left: false, pressed: touch.pressed }), // bJogR
        _ if !touch.pressed => return None,
        _ => {}
    }
    Some(match touch.component {
        7 => Key::Digit(1),
        8 => Key::Digit(2),
        9 => Key::Digit(3),
        11 => Key::Digit(4),
        12 => Key::Digit(5),
        13 => Key::Digit(6),
        14 => Key::Digit(7),
        15 => Key::Digit(8),
        16 => Key::Digit(9),
        17 => Key::Digit(0),
        19 => Key::Point,        // bNumPeriod
        10 => Key::Backspace,    // bBackspace
        18 => Key::Enter,        // bNumOK
        20 => Key::ToggleEngage, // bToggleEngaged (new variant)
        21 => Key::Reverse,      // bReverseToggle
        22 => Key::ToggleUnit,   // bUnitsToggle
        23 => Key::CycleStep,    // bStepCycle
        25 => Key::StepSize(1),  // bPitch001 "0.01"  -- see 4.1
        26 => Key::StepSize(2),  // bPitch01  "0.1"
        27 => Key::StepSize(3),  // bPitch1   "1.0"
        5 => Key::StopLeft,      // bLeftStop
        6 => Key::StopRight,     // bRightStop
        28 => Key::ZeroZ,        // bZeroZ
        31 => Key::JogCycle,     // bCycleJogDist
        33 => Key::Setup,        // bSettings
        3 | 4 => Key::ZeroTurns, // tAngle, tTurns
        _ => return None,
    })
}
```

And the matching `FIELDS`, pending §4:

```rust
pub const FIELDS: [&str; N] = [
    "bToggleEngaged", // status
    "tPitch",
    "bUnitsToggle",   // was bMeasure
    "bStepCycle",     // was tStepVal
    "tRPM",           // was tRPMVal
    "tTurns",         // was tTurnsVal
    "tAngle",         // was tAngleVal
    "tZPos",          // was tZ
    "bLeftStop",      // was tZLeft
    "bRightStop",     // was tZRight
    "bCycleJogDist",  // was tJogVal
    "tSpeed",         // new
    // t3 message line: see 4.2
];
```

`docs/nextion.md` also describes the old H5-derived ids ("ids below 51
are the existing NanoEls H5 ids"). That no longer holds: this HMI
renumbers from 1. Update or replace it once the decisions above are made.
