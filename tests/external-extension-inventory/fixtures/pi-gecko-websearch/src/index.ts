/**
 * pi-gecko-websearch — Web search and browsing via headless Gecko browser.
 * Uses the Marionette protocol to control a real browser with your fingerprint and cookies.
 */

import { StringEnum } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, formatSize, truncateHead } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";
import { BrowserManager } from "./browser.js";
import { parseSearchResults, type SearchResult } from "./parsers.js";

// ---------------------------------------------------------------------------
// Search engine URL builders
// ---------------------------------------------------------------------------

const SEARCH_URLS: Record<string, (q: string) => string> = {
	google: (q) => `https://www.google.com/search?q=${encodeURIComponent(q)}`,
	duckduckgo: (q) => `https://html.duckduckgo.com/html/?q=${encodeURIComponent(q)}`,
	brave: (q) => `https://search.brave.com/search?q=${encodeURIComponent(q)}`,
};

const DEFAULT_ENGINE = "duckduckgo";

function formatResults(results: SearchResult[]): string {
	if (results.length === 0) return "No search results found.";
	return results
		.map((r, i) => {
			let entry = `${i + 1}. ${r.title}\n   ${r.url}`;
			if (r.snippet) entry += `\n   ${r.snippet}`;
			return entry;
		})
		.join("\n\n");
}

function searchFailureDiagnostic(pageText: string, engine: string): string | null {
	const text = pageText.replace(/\s+/g, " ").trim();
	if (!text) return null;

	if (/unusual traffic|not a robot|captcha|verify (?:that )?you|automated queries/i.test(text)) {
		return `${engine} returned an anti-bot/verification page instead of search results:\n\n${text.slice(0, 1000)}`;
	}

	if (/enable javascript|turn on javascript|cookies are disabled/i.test(text)) {
		return `${engine} returned a browser compatibility page instead of search results:\n\n${text.slice(0, 1000)}`;
	}

	return null;
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI) {
	const browser = new BrowserManager();

	const update = (onUpdate: any, msg: string) =>
		onUpdate?.({ content: [{ type: "text", text: msg }], details: undefined });

	const truncate = (content: string, prefix = "") => {
		const t = truncateHead(content, { maxLines: DEFAULT_MAX_LINES, maxBytes: DEFAULT_MAX_BYTES });
		let text = prefix + t.content;
		if (t.truncated)
			text += `\n\n[Truncated: ${t.outputLines}/${t.totalLines} lines, ${formatSize(t.outputBytes)}/${formatSize(t.totalBytes)}]`;
		return text;
	};

	// -------------------------------------------------------------------
	// web_search
	// -------------------------------------------------------------------
	pi.registerTool({
		name: "web_search",
		label: "Web Search",
		description:
			"Search the web using a real Gecko browser. Returns search result titles, URLs, and snippets. Uses your browser fingerprint and cookies.",
		promptSnippet: "Search the web via a Gecko browser (real browser fingerprint + cookies)",
		promptGuidelines: [
			"Use specific, targeted search queries for best results.",
			"Default engine is DuckDuckGo (fastest, most reliable parsing). Use Google if DDG results are insufficient.",
			"The browser uses cookies from the user's configured Gecko profile, so logged-in results may appear.",
			"After searching, use web_browse to read specific result pages.",
		],
		parameters: Type.Object({
			query: Type.String({ description: "Search query" }),
			engine: Type.Optional(
				StringEnum(["google", "duckduckgo", "brave"], {
					description: 'Search engine (default: "duckduckgo")',
				}),
			),
		}),

		renderCall(args: { query: string; engine?: string }, theme: any, context: any) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			const engine = args.engine || DEFAULT_ENGINE;
			let s = theme.fg("toolTitle", theme.bold("web_search "));
			s += theme.fg("muted", `[${engine}] `);
			s += theme.fg("dim", `"${args.query ?? ""}"`);
			text.setText(s);
			return text;
		},

		async execute(_toolCallId, params, signal, onUpdate, _ctx) {
			const engine = params.engine || DEFAULT_ENGINE;
			const buildUrl = SEARCH_URLS[engine];
			if (!buildUrl) {
				throw new Error(`Unknown search engine: "${engine}". Use google, duckduckgo, or brave.`);
			}

			update(onUpdate, "Acquiring browser...");
			return browser.withClient(async (client) => {
				if (signal?.aborted) throw new Error("Aborted");

				const url = buildUrl(params.query);
				update(onUpdate, `Searching ${engine}...`);
				await client.navigate(url, 30_000);
				if (signal?.aborted) throw new Error("Aborted");

				update(onUpdate, "Extracting results...");
				const html = await client.getPageSource(10_000);
				const results = parseSearchResults(html, engine);

				if (results.length === 0) {
					const pageText = await client
						.executeScript(`return document.body?.innerText || document.documentElement?.innerText || ""`, [], 5000)
						.catch(() => "");
					const diagnostic = typeof pageText === "string" ? searchFailureDiagnostic(pageText, engine) : null;
					if (diagnostic) {
						return {
							content: [{ type: "text" as const, text: truncate(diagnostic) }],
							details: { engine, query: params.query, resultCount: 0, blocked: true },
						};
					}
				}

				const formatted = formatResults(results);
				const output = `Search results for "${params.query}" (${engine}, ${results.length} results):\n\n${formatted}`;

				const text = truncate(output);

				return {
					content: [{ type: "text" as const, text }],
					details: { engine, query: params.query, resultCount: results.length },
				};
			});
		},
	});

	// -------------------------------------------------------------------
	// web_browse
	// -------------------------------------------------------------------
	pi.registerTool({
		name: "web_browse",
		label: "Web Browse",
		description:
			"Browse a URL using a real Gecko browser. Returns page content as text. Optionally run a JS extraction script to pull specific data from the page.",
		promptSnippet: "Browse a URL via a Gecko browser and return its content (supports JS extraction)",
		promptGuidelines: [
			"Use web_browse to read a specific page after finding its URL via web_search.",
			"For large pages, provide an `extract` script to get just the relevant content.",
			"The extract parameter is a JS expression evaluated in the page — it must return a string.",
			"Example extract: \"document.querySelector('article')?.innerText\"",
			"Without extract, you get the full page text (HTML stripped), which may be very large.",
		],
		parameters: Type.Object({
			url: Type.String({ description: "URL to navigate to" }),
			extract: Type.Optional(
				Type.String({
					description:
						"JS expression to extract data from the page. Must return a string. Example: \"document.querySelector('article')?.innerText\"",
				}),
			),
		}),

		renderCall(args: { url: string; extract?: string }, theme: any, context: any) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			let s = theme.fg("toolTitle", theme.bold("web_browse "));
			s += theme.fg("muted", args.url ?? "");
			if (args.extract) s += theme.fg("dim", " (extract)");
			text.setText(s);
			return text;
		},

		async execute(_toolCallId, params, signal, onUpdate, _ctx) {
			update(onUpdate, "Acquiring browser...");
			return browser.withClient(async (client) => {
				if (signal?.aborted) throw new Error("Aborted");

				update(onUpdate, `Navigating to ${params.url}...`);
				await client.navigate(params.url, 30_000);
				if (signal?.aborted) throw new Error("Aborted");

				let content: string;

				if (params.extract) {
					update(onUpdate, "Running extraction script...");
					let script = params.extract.trim();
					if (!script.startsWith("return ")) script = `return ${script}`;
					const result = await client.executeScript(script, [], 10_000);
					content = typeof result === "string" ? result : JSON.stringify(result, null, 2);
				} else {
					update(onUpdate, "Extracting page content...");
					content = await client.executeScript(
						`return document.body?.innerText || document.documentElement?.innerText || ""`,
						[],
						10_000,
					);
				}

				const header = `Content from ${params.url} (${formatSize(Buffer.byteLength(content, "utf-8"))}):\n\n`;
				const text = truncate(content, header);

				return {
					content: [{ type: "text" as const, text }],
					details: {
						url: params.url,
						extracted: !!params.extract,
						contentLength: content.length,
					},
				};
			});
		},
	});

	// -------------------------------------------------------------------
	// Lifecycle
	// -------------------------------------------------------------------
	pi.on("session_shutdown", async () => {
		await browser.shutdown();
	});
}
