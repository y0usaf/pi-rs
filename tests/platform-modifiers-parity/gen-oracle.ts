// Generates tests/platform-modifiers-parity/oracle.json by driving Pi's real
// `packages/tui/src/native-modifiers.ts` and `terminal.ts` over a modifier-key
// + Apple-Terminal normalization matrix. Run via scripts/platform-modifiers-oracle.
//
// The native addon is not loadable in this Linux CI base, so `isNativeModifierPressed`
// is exercised exactly as Pi behaves here (helper unavailable => false), and
// `normalizeAppleTerminalInput` is driven over explicit platform/shift states.
import { writeFileSync } from "node:fs";
import { isNativeModifierPressed, type ModifierKey } from "../../ref/pi/packages/tui/src/native-modifiers.ts";
import { normalizeAppleTerminalInput } from "../../ref/pi/packages/tui/src/terminal.ts";

const modifierKeys: ModifierKey[] = ["shift", "command", "control", "option"];

const modifierProbe = modifierKeys.map((key) => ({ key, pressed: isNativeModifierPressed(key) }));

const appleInputs: Array<{ data: string; isAppleTerminal: boolean; isShift: boolean }> = [];
const inputSamples = ["\r", "\n", "a", "\t", "\x1b[13;2u", "\x1b[Z", " "];
for (const data of inputSamples) {
  for (const isAppleTerminal of [true, false]) {
    for (const isShift of [true, false]) {
      appleInputs.push({ data, isAppleTerminal, isShift });
    }
  }
}

const appleNormalized = appleInputs.map(({ data, isAppleTerminal, isShift }) => ({
  data,
  isAppleTerminal,
  isShift,
  out: normalizeAppleTerminalInput(data, isAppleTerminal, isShift),
}));

const out = {
  oracle: "Pi v0.79.0 c5582102",
  platform: process.platform,
  arch: process.arch,
  modifierProbe,
  appleNormalized,
};
writeFileSync(process.argv[2]!, JSON.stringify(out, null, 2));
console.log(`wrote ${process.argv[2]}`);