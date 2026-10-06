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
BOARD = (0.0, 8.4, 51.0, 78.5)

# reference: (x, y, rotation) of pad 1. Terminals are turned so their
# wire openings face the board edge (270 on the left, 90 on the right),
# checked in a 3D render. Parts under the DevKit are low
# (axial resistors flat, DIP without socket): the DevKit sits ~8.5 mm up on
# its sockets.
PLACE = {
    "U1": (14.0, 10.0, 0),
    "U2": (32.0, 40.48, 180),
    "J3": (5.5, 22.7, 270),   # encoder, top to bottom: 5V GND A B Z
    "J6": (5.5, 51.0, 270),   # shield GND
    "J5": (46.0, 27.66, 90),  # Nextion: pin 4 (RX) at the top
    "J2": (45.5, 55.32, 90),  # servo, bottom to top: 5V STEP DIR ENA GND
    "J4": (45.5, 67.08, 90),  # 5 V in, bottom to top: +5V GND
    # ENA: Q1 lies flat (body towards the board's middle), R1/R2 around it.
    "Q1": (26.0, 11.2, 0),
    "R1": (26.0, 22.2, 0),
    "R2": (32.5, 19.0, 90),
    # Encoder: 220R series resistors upright beside the left socket row...
    "R8": (17.6, 30.62, 90),
    "R9": (20.9, 30.62, 90),
    "R10": (17.6, 40.62, 90),
    # ...and the 220p filter caps in the bottom-left corner, below the DevKit.
    "C3": (13.5, 62.8, 0),
    "C4": (13.5, 66.1, 0),
    "C5": (13.5, 69.4, 0),
    "J7": (40.5, 12.0, 90),   # Nextion, Dupont: 5V GND RX TX left to right
    "R3": (16.8, 52.0, 0),
    "R4": (16.8, 55.5, 0),
    "R7": (16.8, 59.0, 0),
    "R5": (27.0, 52.0, 0),
    "R6": (27.0, 55.5, 0),
    # C1 (axial, flat) decouples U2 right at its supply pin; C2 is too tall
    # for under the DevKit, so it sits below it, by the 5 V in.
    "C1": (24.5, 44.2, 0),
    "C2": (31.8, 63.5, 0),
    "D1": (25.0, 69.0, 0),
}


# Ground stitching vias, in spots the parts leave free.
STITCH = [(2.0, 10.0), (49.0, 10.0), (12.0, 76.5), (39.0, 76.5),
          (39.0, 66.0), (35.0, 59.0), (12.0, 47.0), (16.6, 47.2), (49.6, 24.0)]

# M3 mounting holes (non-plated), board-only parts.
HOLES = [(5.5, 13.5), (5.5, 74.4), (45.5, 74.4)]


# Connector pin labels on the silkscreen, by pad number, in the strip between
# each connector and the DevKit sockets: (x of the label column, labels).
PIN_LABELS = {
    "J3": (11.8, ["5V", "GND", "A", "B", "Z"]),
    "J2": (39.1, ["5V", "STEP", "DIR", "ENA", "GND"]),
    "J4": (39.1, ["+5V", "GND"]),
    "J5": (41.6, ["5V", "GND", "RX", "TX"]),  # the display's pins
}
# (text, x, y, rotation)
TITLES = [
    ("ENCODER", 11.8, 15.6, 90),
    ("SHIELD", 11.8, 56.1, 90),
    ("SERVO", 39.1, 30.6, 90),
    ("5V IN", 39.1, 71.0, 0),
    ("NEXTION", 46.0, 16.4, 0),
]


# Reference text nudges where the body centre falls on a pad.
LABEL_OFFSET = {"C2": (3.8, 0), "Q1": (-4.0, 0)}


def mm(v):
    return pcbnew.FromMM(v)


def silk(board, text, x, y, rot, layer=pcbnew.F_SilkS):
    t = pcbnew.PCB_TEXT(board)
    t.SetText(text)
    t.SetLayer(layer)
    t.SetTextSize(pcbnew.VECTOR2I(mm(1.0), mm(1.0)))
    t.SetTextThickness(mm(0.15))
    t.SetTextAngleDegrees(rot)
    t.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    if layer == pcbnew.B_SilkS:
        t.SetMirrored(True)
    board.Add(t)


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

    for ref, (col, labels) in PIN_LABELS.items():
        fp = board.FindFootprintByReference(ref)
        for pad in fp.Pads():
            text = labels[int(pad.GetNumber()) - 1]
            y = pcbnew.ToMM(pad.GetPosition().y)
            # J5's pins are 2.5 mm apart: horizontal labels; terminals are
            # 5.08 mm apart: labels along the edge.
            silk(board, text, col, y, 0 if ref == "J5" else 90)
    for text, x, y, rot in TITLES:
        silk(board, text, x, y, rot)
    # J7's pins run left to right: labels underneath.
    j7 = board.FindFootprintByReference("J7")
    for pad in j7.Pads():
        x = pcbnew.ToMM(pad.GetPosition().x)
        silk(board, ["5V", "GND", "RX", "TX"][int(pad.GetNumber()) - 1], x, 14.4, 0)
    j7.Reference().SetLayer(pcbnew.F_Fab)
    silk(board, "leels breakout rev A", 25.5, 74.2, 0, layer=pcbnew.B_SilkS)
    silk(board, "DevKitC-1 on top, USB down", 25.5, 76.4, 0, layer=pcbnew.B_SilkS)
    for i, (hx, hy) in enumerate(HOLES, 1):
        hole = load_fp("MountingHole:MountingHole_3.2mm_M3")
        hole.SetReference(f"H{i}")
        hole.SetPosition(pcbnew.VECTOR2I(mm(hx), mm(hy)))
        hole.SetBoardOnly(True)
        hole.SetExcludedFromBOM(True)
        hole.Reference().SetLayer(pcbnew.F_Fab)
        board.Add(hole)
    # Connector titles say what each one is; their references would sit on
    # the pin labels.
    for ref in PIN_LABELS:
        board.FindFootprintByReference(ref).Reference().SetLayer(pcbnew.F_Fab)
    board.FindFootprintByReference("J6").Reference().SetLayer(pcbnew.F_Fab)

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
