// Probe used by gen-oracle.ts: in a fresh process under a given env state,
// report whether Pi's module-level `clipboard` resolved the native addon.
// Outputs one line of JSON on stdout.
import { clipboard } from "../../ref/pi/packages/coding-agent/src/utils/clipboard-native.ts";
process.stdout.write(JSON.stringify({ clipboardLoaded: clipboard !== null }));
