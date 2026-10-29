import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	let prompt = "";
	pi.on("before_agent_start", (event) => {
		prompt = event.messageType === "user" ? event.prompt : "";
		return {
			message: { customType: "probe-note", content: `note for ${prompt}`, display: true },
			systemPrompt: `${event.systemPrompt}\nPROBE-MARK`,
		};
	});
	pi.on("turn_end", (event) => {
		if (event.turnIndex !== 0 || prompt !== "continue") return;
		return {
			entries: [
				...event.entries,
				{ type: "custom", customType: "probe-state", data: { n: 1 } },
				{ type: "custom_message", customType: "probe-turn", content: "turn note", display: false },
			],
			continue: true,
		};
	});
	pi.on("agent_start", () => {});
	pi.on("turn_start", () => {});
	pi.on("agent_end", () => {});
	pi.on("agent_settled", () => {});
}
