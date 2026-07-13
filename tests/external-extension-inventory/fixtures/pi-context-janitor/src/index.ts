import type { ToolResultMessage } from "@earendil-works/pi-ai";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { batchFromRecords, captureBatch, pendingHysteresis } from "./capture.js";
import {
	DEFAULT_SETTINGS,
	DEBOUNCE_MS,
	INDEX_CUSTOM_TYPE,
	JANITOR_CUSTOM_TYPES,
	MAX_RECORDS_PER_PASS,
	NOTICE_CUSTOM_TYPE,
	RESTORE_CUSTOM_TYPE,
	STATUS_DISABLED,
	STATUS_ENABLED_IDLE,
	STATUS_KEY,
	STATUS_SPINNER_FRAMES,
	STATUS_SPINNER_MS,
	SUMMARY_CUSTOM_TYPE,
} from "./constants.js";
import { decideRecords } from "./decider.js";
import {
	applyIndexEntry,
	entryFromRun,
	makeRestoreId,
	makeSummaryId,
	parseIndexEntry,
	parseRestoreEntry,
	projectionText,
} from "./index-store.js";
import { loadSettings, saveSettings } from "./settings.js";
import type { CapturedBatch, JanitorSettings, KeybindingsLike, SummaryIndexEntry, ThemeLike, ToolCallRecord } from "./types.js";
import { HiddenMessageComponent, JanitorNoticeComponent, JanitorUndoPicker, janitorRestoreNoticeText, janitorRunNoticeText, undoRunItems } from "./ui.js";
import { isRecord, splitArgs, textFromContent } from "./utils.js";

export default function contextJanitor(pi: ExtensionAPI) {
	// Hide legacy janitor summary custom messages that were emitted by older versions.
	pi.registerMessageRenderer(SUMMARY_CUSTOM_TYPE, () => new HiddenMessageComponent());
	pi.registerMessageRenderer(NOTICE_CUSTOM_TYPE, (message, _state, theme) => {
		const content = typeof message.content === "string" ? message.content : textFromContent(message.content);
		return new JanitorNoticeComponent(content, message.details, theme as ThemeLike);
	});

	let settings: JanitorSettings = { ...DEFAULT_SETTINGS };
	let settingsError: string | undefined;
	let index = new Map<string, ToolCallRecord>();
	let entries = new Map<string, SummaryIndexEntry>();
	let restoredSummaryIds = new Set<string>();

	let pendingBatches: CapturedBatch[] = [];

	let scheduleTimer: ReturnType<typeof setTimeout> | undefined;
	let flushPromise: Promise<void> | undefined;
	let activeController: AbortController | undefined;
	let generation = 0;
	let lastCtx: ExtensionContext | undefined;
	let statusSpinner: ReturnType<typeof setInterval> | undefined;
	let statusSpinnerIndex = 0;

	function abortBackground(): void {
		if (scheduleTimer) clearTimeout(scheduleTimer);
		scheduleTimer = undefined;
		activeController?.abort();
	}

	function statusText(): string {
		if (!settings.enabled) return STATUS_DISABLED;
		if (flushPromise) return `janitor ${STATUS_SPINNER_FRAMES[statusSpinnerIndex]}`;
		return STATUS_ENABLED_IDLE;
	}

	function stopStatusSpinner(): void {
		if (statusSpinner) clearInterval(statusSpinner);
		statusSpinner = undefined;
		statusSpinnerIndex = 0;
	}

	function updateStatus(ctx: ExtensionContext | undefined = lastCtx): void {
		if (!ctx) return;
		lastCtx = ctx;
		const spinning = settings.enabled && !!flushPromise;
		if (spinning && !statusSpinner) {
			statusSpinner = setInterval(() => {
				statusSpinnerIndex = (statusSpinnerIndex + 1) % STATUS_SPINNER_FRAMES.length;
				updateStatus();
			}, STATUS_SPINNER_MS);
			statusSpinner.unref?.();
		} else if (!spinning) {
			stopStatusSpinner();
		}
		ctx.ui.setStatus(STATUS_KEY, statusText());
	}

	function reconstruct(ctx: ExtensionContext): void {
		index = new Map<string, ToolCallRecord>();
		entries = new Map<string, SummaryIndexEntry>();
		restoredSummaryIds = new Set<string>();

		const seenSummaries = new Set<string>();
		for (const entry of ctx.sessionManager.getBranch()) {
			if (!isRecord(entry) || entry.type !== "custom") continue;
			if (entry.customType === INDEX_CUSTOM_TYPE) {
				const parsed = parseIndexEntry(entry.data);
				if (!parsed || seenSummaries.has(parsed.summaryId)) continue;
				seenSummaries.add(parsed.summaryId);
				applyIndexEntry(parsed, index, entries);
			} else if (entry.customType === RESTORE_CUSTOM_TYPE) {
				const parsed = parseRestoreEntry(entry.data);
				if (!parsed) continue;
				for (const summaryId of parsed.summaryIds) restoredSummaryIds.add(summaryId);
			}
		}
	}

	function scheduleFlush(ctx: ExtensionContext, reason: string): void {
		lastCtx = ctx;
		if (!settings.enabled || pendingBatches.length === 0) return;
		if (scheduleTimer) clearTimeout(scheduleTimer);
		const hysteresis = pendingHysteresis(pendingBatches);
		const delayMs = hysteresis.ready ? DEBOUNCE_MS : hysteresis.nextDelayMs;
		scheduleTimer = setTimeout(() => {
			scheduleTimer = undefined;
			const latestHysteresis = pendingHysteresis(pendingBatches);
			if (!latestHysteresis.ready) {
				scheduleFlush(ctx, reason);
				updateStatus(ctx);
				return;
			}
			void flushPending(ctx, `${reason}:${latestHysteresis.reason}`).catch(error => {
				if (ctx.hasUI) ctx.ui.notify(`Context Janitor failed: ${error instanceof Error ? error.message : String(error)}`, "warning");
				updateStatus(ctx);
			});
		}, delayMs);
		scheduleTimer.unref?.();
	}

	async function flushPending(ctx: ExtensionContext, reason: string): Promise<void> {
		lastCtx = ctx;
		if (flushPromise) return flushPromise;
		const runGeneration = generation;
		let failed = false;
		const promise = (async () => {
			if (!settings.enabled || pendingBatches.length === 0) return;

			const batches = pendingBatches;
			pendingBatches = [];
			const allRecords = batches.flatMap(batch => batch.toolCalls).filter(record => !index.has(record.toolCallId));
			const passRecords = allRecords.slice(0, MAX_RECORDS_PER_PASS);
			const restRecords = allRecords.slice(MAX_RECORDS_PER_PASS);
			const restBatch = batchFromRecords(restRecords);
			if (restBatch) pendingBatches.push(restBatch);
			if (passRecords.length === 0) return;

			const controller = new AbortController();
			activeController = controller;
			updateStatus(ctx);

			try {
				const summaryId = makeSummaryId();
				const decided = await decideRecords(ctx, passRecords, controller.signal);
				const selectedRecords = decided.records;
				if (controller.signal.aborted) {
					failed = true;
					const retry = batchFromRecords(passRecords.concat(restRecords));
					if (runGeneration === generation && retry) pendingBatches = [retry, ...pendingBatches.filter(batch => batch !== restBatch)];
					return;
				}

				if (runGeneration !== generation) return;
				if (selectedRecords.length === 0) return;

				const entry = entryFromRun(summaryId, reason, selectedRecords, { usage: decided.usage, modelLabel: decided.modelLabel });
				pi.appendEntry(INDEX_CUSTOM_TYPE, entry);
				applyIndexEntry(entry, index, entries);
				const noticeMessage = {
					customType: NOTICE_CUSTOM_TYPE,
					content: janitorRunNoticeText(entry),
					display: true,
					details: { summaryId: entry.summaryId, rawChars: entry.rawChars, projectedChars: entry.projectedChars, toolCalls: entry.toolCalls.length },
					attribution: "agent",
				} as Parameters<ExtensionAPI["sendMessage"]>[0] & { attribution: "agent" };
				pi.sendMessage(noticeMessage);

			} catch (error) {
				failed = true;
				const retry = batchFromRecords(passRecords.concat(restRecords));
				if (runGeneration === generation && retry) pendingBatches = [retry, ...pendingBatches.filter(batch => batch !== restBatch)];
				if (controller.signal.aborted || runGeneration !== generation) return;
				const message = error instanceof Error ? error.message : String(error);
				if (ctx.hasUI) ctx.ui.notify(`Context Janitor failed: ${message}`, "warning");
			} finally {
				if (activeController === controller) activeController = undefined;
			}
		})().finally(() => {
			if (flushPromise === promise) flushPromise = undefined;
			if (runGeneration !== generation) return;
			updateStatus(ctx);
			if (!failed && settings.enabled && pendingBatches.length > 0) scheduleFlush(ctx, "follow-up");
		});
		flushPromise = promise;
		return promise;
	}

	function restoreSummaryIds(summaryIds: string[], reason: string, ctx?: ExtensionContext): number {
		const uniqueIds = [...new Set(summaryIds.map(id => id.trim()).filter(Boolean))];
		const restorable = uniqueIds.filter(summaryId => entries.has(summaryId) && !restoredSummaryIds.has(summaryId));
		if (restorable.length === 0) return 0;
		const restoreEntry = {
			version: 1 as const,
			restoreId: makeRestoreId(),
			createdAt: new Date().toISOString(),
			reason,
			summaryIds: restorable,
		};
		pi.appendEntry(RESTORE_CUSTOM_TYPE, restoreEntry);
		for (const summaryId of restorable) restoredSummaryIds.add(summaryId);
		const restoreMessage = {
			customType: NOTICE_CUSTOM_TYPE,
			content: janitorRestoreNoticeText(restorable.length),
			display: true,
			details: { restoreId: restoreEntry.restoreId, summaryIds: restorable },
			attribution: "user",
		} as Parameters<ExtensionAPI["sendMessage"]>[0] & { attribution: "user" };
		pi.sendMessage(restoreMessage);
		updateStatus(ctx);
		return restorable.length;
	}

	function restoreListText(): string {
		const items = undoRunItems(entries, restoredSummaryIds);
		if (items.length === 0) return "No janitor runs are currently truncated.";
		return [
			"Restorable janitor runs:",
			...items.map(item => `- ${item.summaryId}: ${item.label} — ${item.description}`),
			"",
			"Run /janitor undo in the interactive TUI to restore selected runs.",
		].join("\n");
	}

	async function openUndoPicker(ctx: ExtensionContext): Promise<void> {
		lastCtx = ctx;
		const items = undoRunItems(entries, restoredSummaryIds);
		if (items.length === 0) {
			ctx.ui.notify("Context Janitor: nothing to restore.", "info");
			return;
		}
		if (!ctx.hasUI) {
			ctx.ui.notify(restoreListText(), "info");
			return;
		}

		let selected: string[] | undefined;
		try {
			selected = await ctx.ui.custom<string[] | undefined>((_tui, theme, keybindings, done) => {
				return new JanitorUndoPicker(items, theme, keybindings as unknown as KeybindingsLike, done);
			}, { overlay: true });
		} catch {
			ctx.ui.notify(restoreListText(), "info");
			return;
		}

		if (!selected || selected.length === 0) {
			ctx.ui.notify("Context Janitor: restore cancelled/no selection.", "info");
			return;
		}
		const count = restoreSummaryIds(selected, "user-undo", ctx);
		ctx.ui.notify(count > 0 ? `Context Janitor restored ${count} run(s). Future model context will include those raw tool outputs again.` : "Context Janitor: selected run(s) were already restored.", "info");
	}

	pi.registerCommand("janitor", {
		description: "Context janitor controls: on, off, undo",
		handler: async (args, ctx) => {
			lastCtx = ctx;
			const sub = splitArgs(args)[0]?.toLowerCase() ?? "";

			try {
				switch (sub) {
					case "on":
						settings = { enabled: true };
						await saveSettings(settings);
						settingsError = undefined;
						updateStatus(ctx);
						if (pendingBatches.length > 0) scheduleFlush(ctx, "manual-on");
						ctx.ui.notify("Context Janitor enabled.", "info");
						return;

					case "off":
						settings = { enabled: false };
						await saveSettings(settings);
						generation += 1;
						abortBackground();
						pendingBatches = [];
						settingsError = undefined;
						updateStatus(ctx);
						ctx.ui.notify("Context Janitor disabled. Raw tool outputs will remain in model context.", "info");
						return;

					case "undo":
						await openUndoPicker(ctx);
						return;

					case "":
						ctx.ui.notify("Usage: /janitor on | off | undo", settingsError ? "warning" : "info");
						return;

					default:
						ctx.ui.notify("Usage: /janitor on | off | undo", "warning");
				}
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				ctx.ui.notify(`Context Janitor: ${message}`, "error");
			}
		},
	});

	pi.on("session_start", async (_event, ctx) => {
		generation += 1;
		abortBackground();
		pendingBatches = [];
		lastCtx = ctx;
		const loaded = await loadSettings();
		settings = loaded.settings;
		settingsError = loaded.error;
		reconstruct(ctx);
		updateStatus(ctx);
		if (settingsError && ctx.hasUI) ctx.ui.notify(settingsError, "warning");
	});

	pi.on("session_tree", async (_event, ctx) => {
		generation += 1;
		abortBackground();
		pendingBatches = [];
		lastCtx = ctx;
		reconstruct(ctx);
		updateStatus(ctx);
	});

	pi.on("model_select", async (_event, ctx) => {
		updateStatus(ctx);
	});

	pi.on("turn_end", async (event, ctx) => {
		lastCtx = ctx;
		if (!settings.enabled) {
			updateStatus(ctx);
			return;
		}
		const batch = captureBatch(event.turnIndex, event.message, event.toolResults as ToolResultMessage[] | undefined, index);
		if (!batch) {
			updateStatus(ctx);
			return;
		}
		pendingBatches.push(batch);
		updateStatus(ctx);
		scheduleFlush(ctx, "turn_end");
	});

	pi.on("agent_end", async (_event, ctx) => {
		lastCtx = ctx;
		if (settings.enabled && pendingBatches.length > 0) scheduleFlush(ctx, "agent_end");
		updateStatus(ctx);
	});

	pi.on("context", async (event) => {
		let changed = false;
		const messages = event.messages.flatMap(message => {
			if (isRecord(message) && message.role === "custom" && typeof message.customType === "string" && JANITOR_CUSTOM_TYPES.has(message.customType)) {
				changed = true;
				return [];
			}

			if (!settings.enabled || !isRecord(message) || message.role !== "toolResult" || typeof message.toolCallId !== "string") return [message];
			const record = index.get(message.toolCallId);
			if (!record || restoredSummaryIds.has(record.summaryId) || !entries.has(record.summaryId)) return [message];

			changed = true;
			return [{ ...message, details: undefined, content: [{ type: "text" as const, text: projectionText(record) }] }];
		});

		if (!changed) return;
		return { messages };
	});

	pi.on("session_shutdown", async (_event, ctx) => {
		generation += 1;
		abortBackground();
		stopStatusSpinner();
		ctx.ui.setStatus(STATUS_KEY, undefined);
	});
}
