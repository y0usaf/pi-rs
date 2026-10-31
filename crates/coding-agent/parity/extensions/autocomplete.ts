import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { AutocompleteItem, AutocompleteProvider } from "@earendil-works/pi-tui";

const PLANETS: AutocompleteItem[] = [
	{ value: "%mercury", label: "%mercury", description: "first" },
	{ value: "%venus", label: "%venus", description: "second" },
	{ value: "%earth", label: "%earth", description: "third" },
];

export default function (pi: ExtensionAPI) {
	pi.on("session_start", (_event, ctx) => {
		ctx.ui.addAutocompleteProvider(
			(current: AutocompleteProvider): AutocompleteProvider => ({
				triggerCharacters: ["%"],
				async getSuggestions(lines, cursorLine, cursorCol, options) {
					const before = (lines[cursorLine] ?? "").slice(0, cursorCol);
					const match = before.match(/(?:^|\s)(%\w*)$/);
					if (!match) return current.getSuggestions(lines, cursorLine, cursorCol, options);
					await new Promise((resolve) => setTimeout(resolve, 30));
					if (options.signal.aborted) return null;
					const items = PLANETS.filter((item) => item.value.startsWith(match[1]));
					return items.length > 0 ? { items, prefix: match[1] } : null;
				},
				applyCompletion(lines, cursorLine, cursorCol, item, prefix) {
					if (!prefix.startsWith("%")) return current.applyCompletion(lines, cursorLine, cursorCol, item, prefix);
					const line = lines[cursorLine] ?? "";
					const start = cursorCol - prefix.length;
					const inserted = `${item.value.slice(1)}🪐 `;
					const next = [...lines];
					next[cursorLine] = line.slice(0, start) + inserted + line.slice(cursorCol);
					return { lines: next, cursorLine, cursorCol: start + inserted.length };
				},
			}),
		);
	});
}
