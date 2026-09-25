# Nextion display files

| File | What |
|---|---|
| `lee-ls.HMI` | Nextion Editor project for the 800×480 NX8048P050-011R-Y display. Edit this. |
| `lee-ls.tft` | The same project compiled by the Nextion Editor, ready to load onto the display. |
| `SHA256SUMS` | Checksums of both, so you can tell the pair belongs together (`shasum -a 256 -c SHA256SUMS`). |

The `.tft` was compiled from this exact `.HMI`: every component's geometry in
the HMI appears in the `.tft`'s compiled page, and its compiled components
are in the HMI's id order (1–33).

**When you change the HMI:** recompile the `.tft`, replace both files
together, and regenerate `SHA256SUMS`. If you added or deleted a component,
the Editor renumbers everything after it: update the ids in
`els-core/src/nextion.rs` (`mod id`) and `docs/nextion.md` to match.

To load the `.tft`, copy it to a FAT32 microSD card, put the card in the
display and power it up, or upload it over serial from the Nextion Editor.
