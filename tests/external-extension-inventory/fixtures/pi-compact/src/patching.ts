import { state } from "./state.js";
import { DEFAULT_PI_COMPACT_SETTINGS, type GapRendering, type ThinkingMode } from "./types.js";
import { cloneGapRendering } from "./shared.js";
import { patchToolExecutionComponent } from "./tool-rendering.js";
import { patchUserMessageComponent } from "./user-rendering.js";
import { patchAssistantMessageComponent } from "./thinking-rendering.js";
import { patchCustomMessageComponent } from "./custom-messages.js";

export async function patchPiCompactComponents(): Promise<boolean> {
  if (state.patchPromise) return state.patchPromise;

  state.patchPromise = (async () => {
    const toolsOk = patchToolExecutionComponent();
    const usersOk = patchUserMessageComponent();
    const assistantOk = patchAssistantMessageComponent();
    const customOk = patchCustomMessageComponent();
    return toolsOk && usersOk && assistantOk && customOk;
  })();

  return state.patchPromise;
}

export function patchErrorDetails(): string {
  const errors = [];
  if (state.lastToolPatchError) errors.push(`tools: ${state.lastToolPatchError}`);
  if (state.lastUserPatchError) errors.push(`user-messages: ${state.lastUserPatchError}`);
  if (state.lastAssistantPatchError) errors.push(`thinking: ${state.lastAssistantPatchError}`);
  if (state.lastCustomPatchError) errors.push(`custom-messages: ${state.lastCustomPatchError}`);
  if (state.lastConfigError) errors.push(`config: ${state.lastConfigError}`);
  return errors.length > 0 ? `\n${errors.join("\n")}` : "";
}

export function formatGapRendering(value: GapRendering): string {
  return value.mode === "normal" || value.mode === "hidden" ? value.mode : `${value.mode}${value.gap ? "+gap" : "+tight"}`;
}

export function statusMessage(): string {
  const toolsStatus = state.lastToolPatchError ? "failed" : formatGapRendering(state.toolRendering);
  const userStatus = state.lastUserPatchError ? "failed" : formatGapRendering(state.userRendering);
  const thinkingStatus = state.lastAssistantPatchError ? "failed" : state.thinkingMode;
  const customStatus = state.lastCustomPatchError ? "failed" : "compact";
  return `pi-compact: tools=${toolsStatus} • user=${userStatus} • thinking=${thinkingStatus} • custom=${customStatus}${patchErrorDetails()}`;
}

export function hasStatusError(): boolean {
  return Boolean(state.lastToolPatchError || state.lastUserPatchError || state.lastAssistantPatchError || state.lastCustomPatchError || state.lastConfigError);
}

export function parseGapRenderingArg(args: string, current: GapRendering, defaultValue: GapRendering): GapRendering | undefined {
  const value = args.trim().toLowerCase();
  if (!value || value === "status") return current;

  switch (value) {
    case "normal":
      return { ...current, mode: "normal" };
    case "borderless":
      return { ...current, mode: "borderless", gap: defaultValue.gap };
    case "borderless-tight":
      return { ...current, mode: "borderless", gap: false };
    case "compact":
      return { ...current, mode: "compact", gap: defaultValue.gap };
    case "compact-tight":
      return { ...current, mode: "compact", gap: false };
    case "hidden":
    case "hide":
    case "off":
      return { ...current, mode: "hidden" };
    case "gap":
      return { ...current, gap: true };
    case "no-gap":
    case "nogap":
      return { ...current, gap: false };
    case "toggle":
      return current.mode === "normal" || current.mode === "hidden" ? cloneGapRendering(defaultValue) : { ...current, mode: "normal" };
    case "cycle":
      if (current.mode === "normal") return { ...current, mode: "borderless", gap: defaultValue.gap };
      if (current.mode === "borderless") return { ...current, mode: "compact", gap: defaultValue.gap };
      if (current.mode === "compact") return { ...current, mode: "hidden" };
      return { ...current, mode: "normal" };
    default:
      return undefined;
  }
}

export function parseThinkingArg(args: string, current: ThinkingMode): ThinkingMode | undefined {
  const value = args.trim().toLowerCase();
  if (!value || value === "status") return current;

  switch (value) {
    case "normal":
      return "normal";
    case "compact":
      return "compact";
    case "hidden":
    case "hide":
    case "off":
      return "hidden";
    case "toggle":
      if (current === "normal") return "compact";
      return current === "compact" ? "hidden" : "compact";
    default:
      return undefined;
  }
}

void patchPiCompactComponents();
