import { type ExtensionAPI, type ExtensionContext } from "@earendil-works/pi-coding-agent";

const ANSI_FG_RESET = "\x1b[39m";
const ANSI_BASE_RGB = [
	[0, 0, 0],
	[128, 0, 0],
	[0, 128, 0],
	[128, 128, 0],
	[0, 0, 128],
	[128, 0, 128],
	[0, 128, 128],
	[192, 192, 192],
	[128, 128, 128],
	[255, 0, 0],
	[0, 255, 0],
	[255, 255, 0],
	[0, 0, 255],
	[255, 0, 255],
	[0, 255, 255],
	[255, 255, 255],
] as const;
const ANSI_CUBE_VALUES = [0, 95, 135, 175, 215, 255] as const;

const INDICATOR = {
	frameMs: 50,
	width: 15,
	maxBirthOffsetMs: 1000,
	runes: "0123456789abcdefABCDEF~!@#$£€%^&*()+=_",
	initialChar: ".",
	hiddenMessage: "\u200B", // zero-width: bypass pi's `||` fallback to "Working..."
} as const;

const STARTUP_FRAMES = Math.ceil(INDICATOR.maxBirthOffsetMs / INDICATOR.frameMs) + 1;
const STARTUP_SWITCH_MS = INDICATOR.maxBirthOffsetMs;
const PRERENDERED_FRAMES = INDICATOR.width * 2;
const GRADIENT_RAMP_WIDTH = INDICATOR.width * 3;

type Theme = ExtensionContext["ui"]["theme"];
type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
type Rgb = { r: number; g: number; b: number };
type Hsl = { h: number; s: number; l: number };
type Gradient = ReturnType<typeof workingGradient>;

const THINKING_COLOR: Record<ThinkingLevel, Parameters<Theme["fg"]>[0]> = {
	off: "thinkingOff",
	minimal: "thinkingMinimal",
	low: "thinkingLow",
	medium: "thinkingMedium",
	high: "thinkingHigh",
	xhigh: "thinkingXhigh",
};

let workingIndicatorTimer: ReturnType<typeof setTimeout> | undefined;
let workingIndicatorGeneration = 0;

const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
const rgb = ([r, g, b]: readonly [number, number, number]): Rgb => ({ r, g, b });

function currentThinkingLevel(pi: ExtensionAPI, ctx: ExtensionContext): ThinkingLevel | undefined {
	if (!ctx.model?.reasoning) return undefined;
	const level = pi.getThinkingLevel();
	return level && level in THINKING_COLOR ? (level as ThinkingLevel) : undefined;
}

function ansi256ToRgb(index: number): Rgb {
	if (index < 16) return rgb(ANSI_BASE_RGB[clamp(index, 0, ANSI_BASE_RGB.length - 1)]!);
	if (index >= 232) {
		const gray = 8 + clamp(index - 232, 0, 23) * 10;
		return { r: gray, g: gray, b: gray };
	}
	const offset = clamp(index - 16, 0, 215);
	return {
		r: ANSI_CUBE_VALUES[Math.floor(offset / 36)]!,
		g: ANSI_CUBE_VALUES[Math.floor(offset / 6) % 6]!,
		b: ANSI_CUBE_VALUES[offset % 6]!,
	};
}

function ansiToRgb(ansi: string): Rgb | undefined {
	const truecolor = ansi.match(/\x1b\[38;2;(\d+);(\d+);(\d+)m/);
	if (truecolor) return { r: Number(truecolor[1]), g: Number(truecolor[2]), b: Number(truecolor[3]) };
	const color256 = ansi.match(/\x1b\[38;5;(\d+)m/);
	return color256 ? ansi256ToRgb(Number(color256[1])) : undefined;
}

function rgbToHsl({ r, g, b }: Rgb): Hsl {
	r /= 255;
	g /= 255;
	b /= 255;
	const max = Math.max(r, g, b);
	const min = Math.min(r, g, b);
	const l = (max + min) / 2;
	if (max === min) return { h: 0, s: 0, l };
	const d = max - min;
	const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
	let h = 0;
	if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
	else if (max === g) h = (b - r) / d + 2;
	else h = (r - g) / d + 4;
	return { h: h * 60, s, l };
}

function hslToRgb({ h, s, l }: Hsl): Rgb {
	const c = (1 - Math.abs(2 * l - 1)) * s;
	const hp = (((h % 360) + 360) % 360) / 60;
	const x = c * (1 - Math.abs((hp % 2) - 1));
	const m = l - c / 2;
	let [r, g, b] = [0, 0, 0];
	if (hp < 1) [r, g, b] = [c, x, 0];
	else if (hp < 2) [r, g, b] = [x, c, 0];
	else if (hp < 3) [r, g, b] = [0, c, x];
	else if (hp < 4) [r, g, b] = [0, x, c];
	else if (hp < 5) [r, g, b] = [x, 0, c];
	else [r, g, b] = [c, 0, x];
	return { r: Math.round((r + m) * 255), g: Math.round((g + m) * 255), b: Math.round((b + m) * 255) };
}

function gradientAnsi(start: Rgb, end: Rgb, index: number, total: number): string {
	const t = Math.min(1, total <= 1 ? 0 : index / (total - 1));
	const a = rgbToHsl(start);
	const b = rgbToHsl(end);
	if (a.s < 0.05) a.h = b.h;
	if (b.s < 0.05) b.h = a.h;
	const hueDelta = ((((b.h - a.h) % 360) + 540) % 360) - 180;
	const saturation = a.s + (b.s - a.s) * t;
	const rgb = hslToRgb({
		h: a.h + hueDelta * t,
		s: Math.min(1, t === 0 || t === 1 ? saturation : Math.max(0.4, saturation * 1.25)),
		l: a.l + (b.l - a.l) * t,
	});
	return `\x1b[38;2;${rgb.r};${rgb.g};${rgb.b}m`;
}

function clearWorkingIndicatorTimer(): void {
	workingIndicatorGeneration++;
	if (workingIndicatorTimer) {
		clearTimeout(workingIndicatorTimer);
		workingIndicatorTimer = undefined;
	}
}

function workingGradient(theme: Theme, thinking: ThinkingLevel | undefined): { accentAnsi: string; accentRgb: Rgb | undefined; endRgb: Rgb | undefined } {
	const accentAnsi = theme.getFgAnsi("accent");
	const accentRgb = ansiToRgb(accentAnsi);
	const endAnsi = theme.getFgAnsi(THINKING_COLOR[thinking ?? "high"]);
	const endRgb = ansiToRgb(endAnsi);
	return { accentAnsi, accentRgb, endRgb };
}

function workingGradientAnsi(gradient: Gradient, index: number): string {
	if (!gradient.accentRgb || !gradient.endRgb) return gradient.accentAnsi;
	const wrapped = ((index % GRADIENT_RAMP_WIDTH) + GRADIENT_RAMP_WIDTH) % GRADIENT_RAMP_WIDTH;
	const segment = Math.floor(wrapped / INDICATOR.width);
	const localIndex = wrapped % INDICATOR.width;
	return segment === 1
		? gradientAnsi(gradient.endRgb, gradient.accentRgb, localIndex, INDICATOR.width)
		: gradientAnsi(gradient.accentRgb, gradient.endRgb, localIndex, INDICATOR.width);
}

function workingCellColors(gradient: Gradient, offset: number): string[] {
	return Array.from({ length: INDICATOR.width }, (_, i) => workingGradientAnsi(gradient, i + offset));
}

function renderWorkingFrame(colors: string[], chars: string[]): string {
	const cells = colors.map((color, i) => `${color}${chars[i] ?? INDICATOR.initialChar}`).join("");
	return `${cells}${ANSI_FG_RESET}`;
}

function randomWorkingChars(): string[] {
	return Array.from(
		{ length: INDICATOR.width },
		() => INDICATOR.runes[Math.floor(Math.random() * INDICATOR.runes.length)] ?? INDICATOR.initialChar,
	);
}

function renderFrames(gradient: Gradient, charFrames: string[][]): string[] {
	return charFrames.map((chars, frame) => renderWorkingFrame(workingCellColors(gradient, frame), chars));
}

function buildWorkingLoopFrames(gradient: Gradient, charFrames = Array.from({ length: PRERENDERED_FRAMES }, randomWorkingChars)): string[] {
	return renderFrames(gradient, charFrames);
}

function buildWorkingStartupFrames(gradient: Gradient, loopCharFrames: string[][]): string[] {
	const birthOffsets = Array.from({ length: INDICATOR.width }, () => Math.random() * INDICATOR.maxBirthOffsetMs);
	return Array.from({ length: STARTUP_FRAMES }, (_, frame) => {
		const elapsedMs = frame * INDICATOR.frameMs;
		const cyclingChars = loopCharFrames[frame % loopCharFrames.length] ?? [];
		const chars = Array.from({ length: INDICATOR.width }, (_, index) =>
			elapsedMs < (birthOffsets[index] ?? 0) ? INDICATOR.initialChar : (cyclingChars[index] ?? INDICATOR.initialChar),
		);
		return renderWorkingFrame(workingCellColors(gradient, frame), chars);
	});
}

function applyWorkingIndicator(pi: ExtensionAPI, ctx: ExtensionContext, startup = false): void {
	clearWorkingIndicatorTimer();
	const gradient = workingGradient(ctx.ui.theme, currentThinkingLevel(pi, ctx));
	const loopCharFrames = Array.from({ length: PRERENDERED_FRAMES }, randomWorkingChars);
	const loopFrames = buildWorkingLoopFrames(gradient, loopCharFrames);
	ctx.ui.setWorkingMessage(INDICATOR.hiddenMessage);

	if (startup) {
		ctx.ui.setWorkingIndicator({ frames: buildWorkingStartupFrames(gradient, loopCharFrames), intervalMs: INDICATOR.frameMs });
		const generation = workingIndicatorGeneration;
		workingIndicatorTimer = setTimeout(() => {
			if (generation !== workingIndicatorGeneration) return;
			workingIndicatorTimer = undefined;
			ctx.ui.setWorkingIndicator({ frames: loopFrames, intervalMs: INDICATOR.frameMs });
		}, STARTUP_SWITCH_MS);
		return;
	}

	ctx.ui.setWorkingIndicator({ frames: loopFrames, intervalMs: INDICATOR.frameMs });
}

export default function workingIndicator(pi: ExtensionAPI) {
	pi.on("session_start", (_event, ctx) => applyWorkingIndicator(pi, ctx));
	pi.on("session_shutdown", (_event, ctx) => {
		clearWorkingIndicatorTimer();
		ctx.ui.setWorkingIndicator();
		ctx.ui.setWorkingMessage();
	});
	pi.on("before_agent_start", (_event, ctx) => applyWorkingIndicator(pi, ctx, true));
	pi.on("model_select", (_event, ctx) => applyWorkingIndicator(pi, ctx));
}
