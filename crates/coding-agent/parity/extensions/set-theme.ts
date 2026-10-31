import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	pi.registerCommand("set-theme", {
		handler: async (args, ctx) => {
			const name = args.trim();
			const result = ctx.ui.setTheme(name);
			const error = result.error === undefined ? "" : ` (${result.error})`;
			ctx.ui.notify(`theme ${name}: ${result.success}${error}, now ${ctx.ui.theme.name}`);
		},
	});
}
