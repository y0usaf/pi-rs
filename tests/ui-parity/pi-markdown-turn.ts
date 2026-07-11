// Marked edge-case transcript scenario (headings, setext, nested lists,
// task lists, quotes, tables, hr, links, emphasis edge cases). Pins terminal
// capabilities so link rendering is environment-independent (no OSC 8),
// matching the pi-rs harness pin, then reuses the basic-turn driver.
import { setCapabilities } from "../../ref/pi/packages/tui/src/terminal-image.ts";
setCapabilities({ images: null, trueColor: true, hyperlinks: false });
// Dynamic import so the pin runs before the driver (tsx compiles to CJS,
// which forbids top-level await; the unawaited import keeps the process
// alive until the driver finishes).
void import("./pi-basic-turn.ts");
