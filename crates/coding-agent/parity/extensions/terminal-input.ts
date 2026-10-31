import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
    pi.registerCommand("input-hooks", {
        handler: async (_args, ctx) => {
            const first = ctx.ui.onTerminalInput((data) => {
                if (data === ":") return { data: "a" };
                if (data === "~") return { consume: true };
                if (data === "?") {
                    ctx.ui.notify(`editor=${ctx.ui.getEditorText()}`);
                    return { consume: true };
                }
            });
            const second = ctx.ui.onTerminalInput((data) => data === "a" ? { data: "b" } : undefined);
            const last = ctx.ui.onTerminalInput((data) => {
                if (data !== "!") return;
                first();
                second();
                last();
                ctx.ui.notify("Input hooks removed");
                return { consume: true };
            });
            ctx.ui.notify("Input hooks ready");
        },
    });
}
