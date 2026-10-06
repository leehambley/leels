"""Build leels-breakout.kicad_pcb from the schematic netlist: board outline,
footprint placement and ground pours. Routing is done separately.

Run with KiCad's Python:
  /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3 build_pcb.py net.xml

net.xml comes from:
  kicad-cli sch export netlist --format kicadxml -o net.xml leels-breakout.kicad_sch
"""

import sys
import xml.etree.ElementTree as ET

import pcbnew

HERE = __file__.rsplit("/", 1)[0]
PCB = f"{HERE}/leels-breakout.kicad_pcb"
KICAD_FP = "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints"
LIBS = {
    "leels": f"{HERE}/leels.pretty",
    "PCM_Espressif": "/Users/leehambley/Documents/KiCad/10.0/3rdparty/footprints/"
    "com_github_espressif_kicad-libraries/Espressif.pretty",
}

# Board: DevKit pin 1 at (14, 10); its PCB spans x 12.5..38.4, y 8.4..60.2
# (antenna beyond the top edge, so no copper under it). Terminals on the
# left and right edges, the 5 V-in diode below the DevKit.
BOARD = (0.0, 8.4, 51.0, 72.0)

# reference: (x, y, rotation) of pad 1. Terminals are turned so their
# wire openings face the board edge (90 on the left, 270 on the right). Parts under the DevKit are low
# (axial resistors flat, DIP without socket): the DevKit sits ~8.5 mm up on
# its sockets.
PLACE = {
    "U1": (14.0, 10.0, 0),
    "U2": (32.0, 40.48, 180),
    "J3": (5.5, 43.02, 90),   # encoder, bottom to top: 5V GND A B Z
    "J6": (5.5, 61.16, 90),   # shield GND
    "J5": (46.0, 27.66, 90),  # Nextion: pin 4 (RX) at the top
    "J2": (45.5, 35.0, 270),  # servo: 5V STEP DIR ENA GND
    "J4": (45.5, 62.0, 270),  # 5 V in
    "Q1": (24.1, 13.0, 0),
    "R1": (33.0, 17.0, 180),
    "R2": (25.38, 20.5, 180),
    "R3": (16.8, 52.0, 0),
    "R4": (16.8, 55.5, 0),
    "R7": (16.8, 59.0, 0),
    "R5": (27.0, 52.0, 0),
    "R6": (27.0, 55.5, 0),
    "C1": (19.5, 45.2, 0),
    "C2": (29.5, 45.9, 0),
    "D1": (25.0, 69.0, 0),
}


# Ground stitching vias, in spots the parts leave free.
STITCH = [(2.0, 10.0), (49.0, 10.0), (2.0, 70.0), (49.0, 70.5), (11.5, 66.0),
          (39.0, 66.0), (21.0, 24.0), (21.0, 40.0), (35.0, 59.0), (12.0, 47.0)]


# Reference text nudges where the body centre falls on a pad.
LABEL_OFFSET = {"C2": (0, -3.4), "Q1": (-4.0, 0)}


def mm(v):
    return pcbnew.FromMM(v)


def load_fp(fpid):
    lib, name = fpid.split(":", 1)
    path = LIBS.get(lib, f"{KICAD_FP}/{lib}.pretty")
    fp = pcbnew.FootprintLoad(path, name)
    if fp is None:
        sys.exit(f"footprint not found: {fpid}")
    fp.SetFPID(pcbnew.LIB_ID(lib, name))
    return fp


def main(netlist):
    root = ET.parse(netlist).getroot()
    board = pcbnew.BOARD()
    board.GetDesignSettings().SetBoardThickness(mm(1.6))

    nets = {}
    pad_net = {}
    for net in root.iter("net"):
        name = net.get("name")
        # KiCad keeps "/" in net names escaped.
        ni = pcbnew.NETINFO_ITEM(board, name.replace("/", "{slash}"))
        board.Add(ni)
        nets[name] = ni
        for node in net.iter("node"):
            pad_net[(node.get("ref"), node.get("pin"))] = ni

    for comp in root.iter("comp"):
        ref = comp.get("ref")
        if ref.startswith("#"):
            continue
        fp = load_fp(comp.findtext("footprint"))
        fp.SetReference(ref)
        # Link to the schematic symbol (schematic parity in DRC).
        sheet = comp.find("sheetpath").get("tstamps")
        fp.SetPath(pcbnew.KIID_PATH(sheet + comp.findtext("tstamps")))
        fp.SetValue(comp.findtext("value"))
        for field in comp.iter("field"):
            name = field.get("name")
            if name in ("Datasheet", "Description"):
                fp.SetField(name, field.text or "")
        x, y, rot = PLACE[ref]
        fp.SetOrientationDegrees(rot)
        board.Add(fp)
        # Place so that pad 1 lands on (x, y).
        p1 = next(p for p in fp.Pads() if p.GetNumber() == "1")
        off = p1.GetPosition() - fp.GetPosition()
        fp.SetPosition(pcbnew.VECTOR2I(mm(x) - off.x, mm(y) - off.y))
        for pad in fp.Pads():
            ni = pad_net.get((ref, pad.GetNumber()))
            if ni is not None:
                pad.SetNet(ni)
        # Dense board: small reference text centred on the part body, and
        # values on the fabrication layer only.
        if ref[0] in "RCDQU" and ref != "U1":
            text = fp.Reference()
            text.SetTextSize(pcbnew.VECTOR2I(mm(0.8), mm(0.8)))
            text.SetTextThickness(mm(0.12))
            text.SetPosition(fp.GetBoundingBox(False, False).GetCenter())
            if ref in LABEL_OFFSET:
                dx, dy = LABEL_OFFSET[ref]
                text.Move(pcbnew.VECTOR2I(mm(dx), mm(dy)))
        fp.Value().SetLayer(pcbnew.F_Fab)

    x0, y0, x1, y1 = BOARD
    rect = pcbnew.PCB_SHAPE(board)
    rect.SetShape(pcbnew.SHAPE_T_RECT)
    rect.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0)))
    rect.SetEnd(pcbnew.VECTOR2I(mm(x1), mm(y1)))
    rect.SetLayer(pcbnew.Edge_Cuts)
    rect.SetWidth(mm(0.1))
    board.Add(rect)

    for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
        zone = pcbnew.ZONE(board)
        zone.SetLayer(layer)
        zone.SetNet(nets["GND"])
        zone.SetLocalClearance(mm(0.3))
        zone.SetMinThickness(mm(0.25))
        zone.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        poly = zone.Outline()
        poly.NewOutline()
        for px, py in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
            poly.Append(mm(px), mm(py))
        board.Add(zone)

    # Stitching vias tie the top and bottom ground pours together, so no
    # pour island is left hanging off a single pad.
    for vx, vy in STITCH:
        via = pcbnew.PCB_VIA(board)
        via.SetPosition(pcbnew.VECTOR2I(mm(vx), mm(vy)))
        via.SetWidth(mm(0.8))
        via.SetDrill(mm(0.4))
        via.SetNet(nets["GND"])
        board.Add(via)

    pcbnew.SaveBoard(PCB, board)
    print(f"wrote {PCB}: {len(board.GetFootprints())} footprints, {len(nets)} nets")


if __name__ == "__main__":
    main(sys.argv[1])
