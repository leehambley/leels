"""Route leels-breakout.kicad_pcb with Freerouting (Specctra DSN/SES).

  python3 route.py export board.dsn     # KiCad Python
  java -jar freerouting-1.9.0.jar -de board.dsn -do board.ses -mp 100
  python3 route.py import board.ses     # KiCad Python
"""
import sys

import pcbnew

PCB = __file__.rsplit("/", 1)[0] + "/leels-breakout.kicad_pcb"
board = pcbnew.LoadBoard(PCB)
if sys.argv[1] == "export":
    # Pours are refilled after routing; the router routes GND as tracks too.
    ok = pcbnew.ExportSpecctraDSN(board, sys.argv[2])
else:
    ok = pcbnew.ImportSpecctraSES(board, sys.argv[2])
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    pcbnew.SaveBoard(PCB, board)
print(sys.argv[1], "ok" if ok else "FAILED")
