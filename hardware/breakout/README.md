# Breakout board

A carrier for the ESP32-C6-DevKitC-1: the DevKit plugs into two 1x16 female
sockets (rows 22.86 mm apart) and sits about 8.5 mm above this board. All parts
are through-hole. The buffer IC, transistor, resistors and capacitors sit under
the DevKit, so keep them low: no IC socket, resistors flat.

- Left edge: encoder terminal (bottom to top: 5V, GND, A, B, Z) and shield
  GND terminal.
- Right edge: Nextion (JST-XH 4), servo terminal (5V, STEP, DIR, ENA, GND) and
  5 V in.
- The board stops where the DevKit's PCB stops, so nothing is under its
  antenna.

Firmware: the normal C6 build (`cargo run --release`), not `--features breadboard`.

## Regenerating the PCB

The schematic is the source. The board is rebuilt from it by script, then
routed with [Freerouting](https://github.com/freerouting/freerouting) 1.9.0:

```sh
KP=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3
kicad-cli sch export netlist --format kicadxml -o net.xml leels-breakout.kicad_sch
$KP build_pcb.py net.xml         # outline, placement, GND pours, stitching vias
$KP route.py export board.dsn
java -jar freerouting-1.9.0.jar -de board.dsn -do board.ses -mp 100 -inc Ground
$KP route.py import board.ses    # also refills the pours
kicad-cli pcb drc --schematic-parity --refill-zones leels-breakout.kicad_pcb
```

Placement lives in `build_pcb.py` (`PLACE`). GND is not routed; the pours on
both layers and the stitching vias carry it. Net classes (0.3 mm signals,
0.6 mm power and ground) are in `leels-breakout.kicad_pro`.

`leels.pretty/ESP32-C6-DevKitC-1_Sockets` is Espressif's DevKitC-1 footprint
with its courtyard limited to the socket rows, so parts may sit under the
DevKit.
