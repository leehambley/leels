"""Route leels-breakout.kicad_pcb with Freerouting (Specctra DSN/SES).

  python3 route.py export board.dsn     # KiCad Python
  java -jar freerouting-1.9.0.jar -de board.dsn -do board.ses -mp 100 -inc Ground
  python3 route.py import board.ses     # KiCad Python

Standalone pcbnew doesn't apply the project's net classes to the DSN export
(everything comes out 0.2 mm), so the export rewrites the DSN's net classes
itself from NET_CLASSES below.
"""
import re
import sys

import pcbnew

PCB = __file__.rsplit("/", 1)[0] + "/leels-breakout.kicad_pcb"

# DSN units are um. (name, track width, clearance, nets or None for the rest)
NET_CLASSES = [
    ("Power", 600, 250, ["+3V3", "+5V", "Net-(D1-A)"]),
    ("Ground", 600, 250, ["GND"]),  # not routed: the pours carry it
    ("kicad_default", 300, 250, None),
]
VIA = "Via[0-1]_800:400_um"


def block_end(text, start):
    """Index just past the parenthesised block starting at text[start]."""
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return i + 1
    raise ValueError("unbalanced DSN")


def rewrite_classes(path):
    dsn = open(path).read()
    all_nets = []
    while (m := re.search(r"\n\s*\(class ", dsn)) is not None:
        start = m.start() + 1
        end = block_end(dsn, dsn.index("(", start))
        names = dsn[start:end].split("(circuit")[0]
        all_nets += re.findall(r'"[^"]*"|[^\s()]+', names)[2:]
        dsn = dsn[:start] + dsn[end:]
    assigned = {n for _, _, _, nets in NET_CLASSES if nets for n in nets}
    classes = []
    for name, width, clearance, nets in NET_CLASSES:
        members = nets if nets else [n for n in all_nets if n.strip('"') not in assigned]
        quoted = " ".join(n if n.startswith('"') or "(" not in n else f'"{n}"' for n in members)
        classes.append(
            f'    (class {name} {quoted}\n      (circuit (use_via "{VIA}"))\n'
            f"      (rule (width {width}) (clearance {clearance}))\n    )\n"
        )
    net_end = block_end(dsn, dsn.index("(network"))
    dsn = dsn[: net_end - 1] + "".join(classes) + dsn[net_end - 1 :]
    # Board-wide default rule.
    dsn = re.sub(r"\(rule\s*\(width \d+\)\s*\(clearance \d+\)", "(rule (width 300) (clearance 250)", dsn, count=1)
    open(path, "w").write(dsn)


board = pcbnew.LoadBoard(PCB)
if sys.argv[1] == "export":
    ok = pcbnew.ExportSpecctraDSN(board, sys.argv[2])
    rewrite_classes(sys.argv[2])
else:
    ok = pcbnew.ImportSpecctraSES(board, sys.argv[2])
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    pcbnew.SaveBoard(PCB, board)
print(sys.argv[1], "ok" if ok else "FAILED")
