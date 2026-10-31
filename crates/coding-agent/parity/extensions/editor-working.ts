import { CustomEditor, type ExtensionAPI } from "@earendil-works/pi-coding-agent";

class MarkedEditor extends CustomEditor {
	render(width: number): string[] {
		return [...super.render(width), "  marked editor"];
	}
}

export default function (pi: ExtensionAPI) {
	let release: (() => void) | undefined;
	pi.on("session_start", (_event, ctx) => {
		ctx.ui.setWorkingIndicator({ frames: ["●"] });
		ctx.ui.setEditorComponent((tui, theme, keybindings) => new MarkedEditor(tui, theme, keybindings));
		ctx.ui.onTerminalInput((data) => {
			if (data !== "\x07" || !release) return;
			release();
			release = undefined;
			return { consume: true };
		});
	});
	pi.on("context", async () => {
		await new Promise<void>((resolve) => {
			release = resolve;
		});
	});
}
