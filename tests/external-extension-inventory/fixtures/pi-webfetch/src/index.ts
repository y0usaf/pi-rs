/**
 * pi-webfetch — Fetch URLs and return content as markdown.
 * No secondary model. No domain blocklist. Just fetch → convert → return.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, formatSize, truncateHead } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";

// ---------------------------------------------------------------------------
// LRU Cache — simple Map with TTL, max 50 entries, 15-min expiry
// ---------------------------------------------------------------------------

interface CacheEntry {
	content: string;
	contentType: string;
	bytes: number;
	timestamp: number;
}

const CACHE_TTL_MS = 15 * 60 * 1000;
const CACHE_MAX = 50;
const cache = new Map<string, CacheEntry>();

function cacheGet(key: string): CacheEntry | undefined {
	const entry = cache.get(key);
	if (!entry) return undefined;
	if (Date.now() - entry.timestamp > CACHE_TTL_MS) {
		cache.delete(key);
		return undefined;
	}
	cache.delete(key);
	cache.set(key, entry);
	return entry;
}

function cacheSet(key: string, entry: CacheEntry): void {
	while (cache.size >= CACHE_MAX) {
		const oldest = cache.keys().next().value;
		if (oldest !== undefined) cache.delete(oldest);
		else break;
	}
	cache.set(key, entry);
}

// ---------------------------------------------------------------------------
// Turndown — lazy singleton
// ---------------------------------------------------------------------------

let td: any;
async function getTurndown() {
	return td ??= new (await import("turndown")).default();
}

// ---------------------------------------------------------------------------
// Fetch with same-host redirect following
// ---------------------------------------------------------------------------

const MAX_REDIRECTS = 5;
const FETCH_TIMEOUT_MS = 30_000;

function isLocalOrPrivateHost(hostname: string): boolean {
	const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
	return (
		host === "localhost" ||
		host.endsWith(".localhost") ||
		host.endsWith(".local") ||
		host === "::1" ||
		host === "0:0:0:0:0:0:0:1" ||
		/^127\./.test(host) ||
		/^10\./.test(host) ||
		/^192\.168\./.test(host) ||
		/^172\.(1[6-9]|2\d|3[01])\./.test(host)
	);
}

async function safeFetch(url: string, signal?: AbortSignal): Promise<Response> {
	let current = url;

	for (let i = 0; i <= MAX_REDIRECTS; i++) {
		const signals = signal
			? [signal, AbortSignal.timeout(FETCH_TIMEOUT_MS)]
			: [AbortSignal.timeout(FETCH_TIMEOUT_MS)];
		const res = await fetch(current, {
			redirect: "manual",
			signal: AbortSignal.any(signals),
			headers: {
				Accept: "text/html, text/markdown, text/plain, */*",
				"User-Agent": "pi-webfetch/1.0",
			},
		});

		if (![301, 302, 307, 308].includes(res.status)) return res;

		const location = res.headers.get("location");
		if (!location) return res;

		const redirectUrl = new URL(location, current);
		const originalHost = new URL(current).hostname.replace(/^www\./, "");
		const redirectHost = redirectUrl.hostname.replace(/^www\./, "");

		if (originalHost !== redirectHost) {
			// Cross-host redirect — return info instead of following
			const body = `Redirect to different host detected.\nOriginal: ${current}\nRedirect: ${redirectUrl.href}\n\nUse web_fetch again with the redirect URL to follow.`;
			return new Response(body, { status: res.status, statusText: res.statusText });
		}

		current = redirectUrl.href;
	}

	throw new Error(`Too many redirects (>${MAX_REDIRECTS})`);
}

// ---------------------------------------------------------------------------
// Extension
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI) {
	pi.registerTool({
		name: "web_fetch",
		label: "Web Fetch",
		description:
			"Fetch a URL and return its content as markdown. For HTML pages, converts to clean markdown. For other content types, returns raw text. Includes a 15-minute cache.",
		promptSnippet: "Fetch a URL and return its content as markdown",
		promptGuidelines: [
			"Use web_fetch to read documentation, API references, or other web content. URL must start with http:// or https:// (public http is tried as https first; localhost/private http stays http). HTML is converted to markdown; non-HTML is returned as-is.",
			"For GitHub repos prefer `gh` CLI via bash. Authenticated pages won't work. Optionally pass `prompt` to indicate what you're looking for.",
		],
		parameters: Type.Object({
			url: Type.String({ description: "Full URL to fetch (http/https)" }),
			prompt: Type.Optional(
				Type.String({
					description: "Optional: what you're looking for in this page (prepended to output for context)",
				}),
			),
		}),

		renderCall(args: { url: string; prompt?: string }, theme: any, context: any) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			let s = theme.fg("toolTitle", theme.bold("web_fetch "));
			s += theme.fg("muted", args.url);
			text.setText(s);
			return text;
		},

		async execute(_toolCallId, params, signal, onUpdate, _ctx) {
			const { url, prompt } = params;

			// Validate
			let parsed: URL;
			try {
				parsed = new URL(url);
			} catch {
				throw new Error(`Invalid URL: "${url}"`);
			}
			if (!["http:", "https:"].includes(parsed.protocol)) {
				throw new Error(`Unsupported protocol: ${parsed.protocol}. Use http or https.`);
			}

			const originalUrl = parsed.href;
			let fetchUrl = parsed.href;
			let fallbackUrl: string | undefined;

			// Try HTTPS for public HTTP URLs, but do not break local/private HTTP endpoints.
			if (parsed.protocol === "http:" && !isLocalOrPrivateHost(parsed.hostname)) {
				parsed.protocol = "https:";
				fetchUrl = parsed.href;
				fallbackUrl = originalUrl;
			}

			// Check cache
			let cachedUrl = fetchUrl;
			let cached = cacheGet(fetchUrl);
			if (!cached && fallbackUrl) {
				const fallbackCached = cacheGet(fallbackUrl);
				if (fallbackCached) {
					cached = fallbackCached;
					cachedUrl = fallbackUrl;
				}
			}
			if (cached) {
				return buildResult(cached.content, cached.bytes, cached.contentType, cachedUrl, prompt, true);
			}

			onUpdate?.({ content: [{ type: "text", text: `Fetching ${fetchUrl}...` }], details: undefined });

			let lastAttemptUrl = fetchUrl;
			try {
				let effectiveUrl = fetchUrl;
				let res: Response;
				try {
					lastAttemptUrl = fetchUrl;
					res = await safeFetch(fetchUrl, signal);
				} catch (err) {
					if (!fallbackUrl || signal?.aborted) throw err;
					onUpdate?.({
						content: [{ type: "text", text: `HTTPS fetch failed; retrying ${fallbackUrl}...` }],
						details: undefined,
					});
					effectiveUrl = fallbackUrl;
					lastAttemptUrl = fallbackUrl;
					res = await safeFetch(fallbackUrl, signal);
				}

				if (!res.ok && ![301, 302, 307, 308].includes(res.status)) {
					throw new Error(`HTTP ${res.status} ${res.statusText}`);
				}

				const contentType = res.headers.get("content-type") ?? "text/plain";
				const rawText = await res.text();
				const bytes = Buffer.byteLength(rawText, "utf-8");

				let content: string;
				if (contentType.includes("text/html")) {
					onUpdate?.({ content: [{ type: "text", text: "Converting HTML to markdown..." }], details: undefined });
					const td = await getTurndown();
					content = td.turndown(rawText);
				} else {
					content = rawText;
				}

				// Cache it
				cacheSet(effectiveUrl, { content, contentType, bytes, timestamp: Date.now() });

				return buildResult(content, bytes, contentType, effectiveUrl, prompt, false);
			} catch (err: unknown) {
				if (err instanceof Error && err.name === "TimeoutError") {
					throw new Error(`Fetch timed out after ${FETCH_TIMEOUT_MS / 1000}s for ${lastAttemptUrl}`);
				}
				throw err;
			}
		},
	});
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function buildResult(
	content: string,
	bytes: number,
	contentType: string,
	url: string,
	prompt: string | undefined,
	fromCache: boolean,
) {
	const truncation = truncateHead(content, {
		maxLines: DEFAULT_MAX_LINES,
		maxBytes: DEFAULT_MAX_BYTES,
	});

	let text = "";
	if (prompt) {
		text += `Looking for: ${prompt}\n\n`;
	}
	text += `URL: ${url}\nSize: ${formatSize(bytes)} | Type: ${contentType}${fromCache ? " (cached)" : ""}\n\n`;
	text += truncation.content;

	if (truncation.truncated) {
		text += `\n\n[Truncated: showing ${truncation.outputLines} of ${truncation.totalLines} lines`;
		text += ` (${formatSize(truncation.outputBytes)} of ${formatSize(truncation.totalBytes)})]`;
	}

	return {
		content: [{ type: "text" as const, text }],
		details: {
			url,
			bytes,
			contentType,
			fromCache,
			truncated: truncation.truncated,
		},
	};
}
