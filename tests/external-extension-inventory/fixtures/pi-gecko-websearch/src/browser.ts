import { getAgentDir } from "@earendil-works/pi-coding-agent";
import { type ChildProcess, spawn, spawnSync } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { MarionetteClient } from "./marionette.js";

interface GeckoWebsearchSettings {
	binary?: string;
	profile?: string;
	profileRoot?: string;
	maxBrowsers?: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function str(v: unknown): string | undefined {
	return typeof v === "string" ? v.trim() || undefined : undefined;
}

function positiveInt(v: unknown): number | undefined {
	const n = typeof v === "number" ? v : typeof v === "string" ? Number.parseInt(v.trim(), 10) : NaN;
	return Number.isInteger(n) && n > 0 ? n : undefined;
}

function pickSettings(parsed: Record<string, unknown>): unknown {
	const extensionSettings = parsed.extensionSettings;
	if (!isRecord(extensionSettings)) return undefined;
	return extensionSettings["gecko-websearch"];
}

function readSettings(filePath: string): GeckoWebsearchSettings {
	if (!fs.existsSync(filePath)) return {};

	try {
		const parsed = JSON.parse(fs.readFileSync(filePath, "utf-8")) as unknown;
		if (!isRecord(parsed)) return {};

		const settings = pickSettings(parsed);
		if (!isRecord(settings)) return {};

		return {
			binary: str(settings.binary),
			profile: str(settings.profile),
			profileRoot: str(settings.profileRoot),
			maxBrowsers: positiveInt(settings.maxBrowsers),
		};
	} catch (error) {
		console.error(
			`[gecko-websearch] Failed to load ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
		);
		return {};
	}
}

function clampMaxBrowsers(value: number | undefined): number {
	return Math.max(1, Math.min(value ?? 2, 8));
}

interface BrowserLease {
	resolve: (browser: ManagedBrowser) => void;
	reject: (error: Error) => void;
}

/**
 * Small browser pool. Each leased browser is exclusive for a full search/browse
 * operation, so parallel tool calls cannot clobber each other's navigation state
 * or Marionette request handlers.
 */
export class BrowserManager {
	private readonly settings: GeckoWebsearchSettings;
	private readonly maxBrowsers: number;
	private readonly browsers = new Set<ManagedBrowser>();
	private readonly idle: ManagedBrowser[] = [];
	private readonly waiters: BrowserLease[] = [];
	private shuttingDown = false;

	constructor(cwd: string = process.cwd()) {
		const globalSettings = readSettings(path.join(getAgentDir(), "settings.json"));
		const projectSettings = readSettings(path.join(cwd, ".pi", "settings.json"));
		this.settings = { ...globalSettings, ...projectSettings };
		this.maxBrowsers = clampMaxBrowsers(positiveInt(process.env.PI_GECKO_MAX_BROWSERS) ?? this.settings.maxBrowsers);
	}

	async withClient<T>(fn: (client: MarionetteClient) => Promise<T>): Promise<T> {
		const browser = await this.acquire();
		try {
			const client = await browser.ensureRunning();
			return await fn(client);
		} finally {
			this.release(browser);
		}
	}

	private acquire(): Promise<ManagedBrowser> {
		if (this.shuttingDown) throw new Error("Gecko browser pool is shutting down");

		const idle = this.idle.pop();
		if (idle) return Promise.resolve(idle);

		if (this.browsers.size < this.maxBrowsers) {
			const browser = new ManagedBrowser(this.settings);
			this.browsers.add(browser);
			return Promise.resolve(browser);
		}

		return new Promise((resolve, reject) => {
			this.waiters.push({ resolve, reject });
		});
	}

	private release(browser: ManagedBrowser): void {
		if (this.shuttingDown || !this.browsers.has(browser)) return;

		const waiter = this.waiters.shift();
		if (waiter) {
			waiter.resolve(browser);
		} else {
			this.idle.push(browser);
		}
	}

	async shutdown(): Promise<void> {
		this.shuttingDown = true;

		for (const waiter of this.waiters.splice(0)) {
			waiter.reject(new Error("Gecko browser pool shut down"));
		}

		const browsers = [...this.browsers];
		this.idle.length = 0;
		this.browsers.clear();

		await Promise.allSettled(browsers.map((browser) => browser.shutdown()));
		this.shuttingDown = false;
	}
}

/** Manages one headless Gecko browser instance with Marionette enabled. */
class ManagedBrowser {
	private process: ChildProcess | null = null;
	private client: MarionetteClient | null = null;
	private tempProfileDir: string | null = null;
	private marionettePort: number | null = null;
	private running = false;

	constructor(private readonly settings: GeckoWebsearchSettings) {}

	/** Lazy-init: if browser isn't running, start it and connect Marionette. */
	async ensureRunning(): Promise<MarionetteClient> {
		if (this.running && this.client?.isConnected) {
			return this.client;
		}

		// Clean up any previous state
		await this.shutdown();

		try {
			// 1. Create temp profile directory
			this.tempProfileDir = fs.mkdtempSync(path.join(os.tmpdir(), "pi-gecko-"));

			// 2. Copy cookies from user's real profile
			const sourceProfile = this.resolveProfilePath();
			if (sourceProfile) {
				this.copyCookies(sourceProfile, this.tempProfileDir);
			}

			// 3. Write a user.js to the temp profile to configure Marionette
			const userJs = [
				'user_pref("marionette.port", 0);',
				'user_pref("marionette.enabled", true);',
				// Disable first-run stuff
				'user_pref("browser.shell.checkDefaultBrowser", false);',
				'user_pref("browser.startup.homepage_override.mstone", "ignore");',
				'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
				'user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);',
				// Disable session restore prompts
				'user_pref("browser.sessionstore.resume_from_crash", false);',
				// Reduce resource usage
				'user_pref("browser.cache.disk.enable", false);',
				'user_pref("media.hardware-video-decoding.enabled", false);',
			].join("\n");
			fs.writeFileSync(path.join(this.tempProfileDir, "user.js"), userJs);

			// 4. Find the Gecko browser binary
			const binary = this.findBinary();

			// 5. Spawn headless Gecko browser with Marionette
			const args = ["--marionette", "--headless", "--profile", this.tempProfileDir, "--no-remote"];
			let startupError: Error | null = null;

			this.process = spawn(binary, args, {
				stdio: "ignore",
				detached: false,
			});

			this.process.once("error", (error) => {
				startupError = error instanceof Error ? error : new Error(String(error));
				this.running = false;
			});

			this.process.on("exit", (code, signal) => {
				if (!this.running && !startupError) {
					const status = signal ? `signal ${signal}` : `code ${code ?? "unknown"}`;
					startupError = new Error(`Gecko browser exited before Marionette became ready (${status})`);
				}
				this.running = false;
			});

			// 6. Wait for Marionette port to be ready, then connect
			this.client = new MarionetteClient();
			await this.waitForMarionette(this.client, this.tempProfileDir, 45000, () => startupError);

			// 7. Create a session
			await this.client.newSession();

			this.running = true;
			return this.client;
		} catch (error) {
			await this.shutdown();
			throw error;
		}
	}

	/** Read Gecko's chosen Marionette port from the temp profile. */
	private readActivePort(profileDir: string): number | null {
		try {
			const text = fs.readFileSync(path.join(profileDir, "MarionetteActivePort"), "utf-8").trim();
			const port = Number.parseInt(text, 10);
			return Number.isInteger(port) && port > 0 ? port : null;
		} catch {
			return null;
		}
	}

	/** Wait for Gecko to publish and accept its Marionette port, retrying. */
	private async waitForMarionette(
		client: MarionetteClient,
		profileDir: string,
		timeoutMs: number,
		getStartupError?: () => Error | null,
	): Promise<void> {
		const start = Date.now();
		const retryDelay = 500;
		let lastPort: number | null = null;

		while (Date.now() - start < timeoutMs) {
			const startupError = getStartupError?.();
			if (startupError) throw new Error(`Failed to start Gecko browser: ${startupError.message}`);

			const port = this.readActivePort(profileDir);
			if (port) {
				lastPort = port;
				try {
					await client.connect(port, "127.0.0.1", 2000);
					this.marionettePort = port;
					return;
				} catch {
					// Port is published but not accepting yet — wait and retry.
				}
			}

			await new Promise((r) => setTimeout(r, retryDelay));
		}

		const suffix = lastPort ? ` on port ${lastPort}` : " (no MarionetteActivePort file)";
		throw new Error(`Timed out waiting for Marionette${suffix} after ${timeoutMs}ms`);
	}

	/**
	 * Resolve the Gecko profile path.
	 * Priority: PI_GECKO_PROFILE env → settings profile → PI_GECKO_PROFILE_ROOT env
	 * → settings profileRoot → auto-detect Firefox/LibreWolf roots.
	 */
	private resolveProfilePath(): string | null {
		const configuredProfile = process.env.PI_GECKO_PROFILE || this.settings.profile;
		if (configuredProfile && fs.existsSync(configuredProfile)) {
			return configuredProfile;
		}

		const home = os.homedir();
		const profileRoots = [
			process.env.PI_GECKO_PROFILE_ROOT,
			this.settings.profileRoot,
			path.join(home, ".mozilla", "firefox"),
			path.join(home, ".librewolf"),
		];

		for (const profileRoot of profileRoots) {
			const profile = this.resolveProfileRoot(profileRoot);
			if (profile) return profile;
		}

		return null;
	}

	private resolveProfileRoot(profileRoot: string | undefined): string | null {
		if (!profileRoot || !fs.existsSync(profileRoot)) return null;

		const profilesIni = path.join(profileRoot, "profiles.ini");
		if (fs.existsSync(profilesIni)) {
			const parsed = this.parseProfilesIni(profilesIni);
			if (parsed) return parsed;
		}

		return this.scanProfileRoot(profileRoot);
	}

	private scanProfileRoot(profileRoot: string): string | null {
		if (fs.existsSync(path.join(profileRoot, "cookies.sqlite"))) {
			return profileRoot;
		}

		try {
			const entries = fs.readdirSync(profileRoot, { withFileTypes: true })
				.filter((e) => e.isDirectory())
				.sort((a, b) => +b.name.includes(".default") - +a.name.includes(".default"));
			for (const entry of entries) {
				const candidate = path.join(profileRoot, entry.name);
				if (fs.existsSync(path.join(candidate, "cookies.sqlite"))) return candidate;
			}
		} catch {
			// ignore
		}

		return null;
	}

	private parseProfilesIni(iniPath: string): string | null {
		const content = fs.readFileSync(iniPath, "utf-8");
		const baseDir = path.dirname(iniPath);
		const sections = content.split(/^\s*\[/m).slice(1);

		let firstProfile: string | null = null;
		let defaultProfile: string | null = null;

		for (const section of sections) {
			if (!/^profile/i.test(section)) continue;
			const kv = Object.fromEntries(
				section.split("\n").filter((l) => l.includes("=")).map((l) => {
					const i = l.indexOf("=");
					return [l.substring(0, i).trim(), l.substring(i + 1).trim()];
				}),
			);
			if (!kv.Path) continue;
			const resolved = kv.IsRelative === "0" ? kv.Path : path.join(baseDir, kv.Path);
			firstProfile ??= resolved;
			if (kv.Default === "1") defaultProfile = resolved;
		}

		const chosen = defaultProfile ?? firstProfile;
		return chosen && fs.existsSync(chosen) ? chosen : null;
	}

	/** Copy cookie DB files (and cert9.db if present) from source to dest profile. */
	private copyCookies(sourceProfile: string, destProfile: string): void {
		const filesToCopy = ["cookies.sqlite", "cookies.sqlite-wal", "cert9.db"];
		for (const file of filesToCopy) {
			const src = path.join(sourceProfile, file);
			if (fs.existsSync(src)) {
				try {
					fs.copyFileSync(src, path.join(destProfile, file));
				} catch {
					// Non-fatal: we can still browse without cookies
				}
			}
		}
	}

	/** Find the Gecko browser binary. */
	private findBinary(): string {
		const configuredCandidates = [process.env.PI_GECKO_BINARY, this.settings.binary];
		for (const candidate of configuredCandidates) {
			const resolved = this.resolveBinaryCandidate(candidate);
			if (resolved) return resolved;
		}

		const names = ["firefox", "librewolf"];
		const prefixes = ["/usr/bin", "/usr/local/bin", "/snap/bin"];
		const flatpaks: Record<string, string> = {
			firefox: "org.mozilla.firefox",
			librewolf: "io.gitlab.librewolf-community",
		};
		const candidates = names.flatMap((n) => [
			n,
			...prefixes.map((p) => `${p}/${n}`),
			`/var/lib/flatpak/exports/bin/${flatpaks[n]}`,
			path.join(os.homedir(), `.local/bin/${n}`),
			`/Applications/${n.charAt(0).toUpperCase() + n.slice(1)}.app/Contents/MacOS/${n}`,
		]);

		for (const candidate of candidates) {
			const resolved = this.resolveBinaryCandidate(candidate);
			if (resolved) return resolved;
		}

		// Fallback: just try "firefox" and let spawn fail with a clear error
		return "firefox";
	}

	private resolveBinaryCandidate(candidate: string | undefined): string | null {
		if (!candidate) return null;
		const value = candidate.trim();
		if (!value) return null;

		if (value.includes("/") || value.includes(path.sep)) {
			return fs.existsSync(value) ? value : null;
		}

		const command = process.platform === "win32" ? "where" : "which";
		const result = spawnSync(command, [value], {
			encoding: "utf-8",
			stdio: ["ignore", "pipe", "ignore"],
		});
		if (result.status !== 0) return null;

		const resolved = result.stdout.trim().split(/\r?\n/, 1)[0];
		return resolved || null;
	}

	/** Shut down the browser and clean up. */
	async shutdown(): Promise<void> {
		if (this.client) {
			try {
				await this.client.close();
			} catch {
				// ignore
			}
			this.client = null;
		}

		if (this.process) {
			try {
				this.process.kill("SIGTERM");
				// Give it a moment, then SIGKILL if needed
				await new Promise<void>((resolve) => {
					const timer = setTimeout(() => {
						try {
							this.process?.kill("SIGKILL");
						} catch {
							// ignore
						}
						resolve();
					}, 3000);

					this.process!.once("exit", () => {
						clearTimeout(timer);
						resolve();
					});
				});
			} catch {
				// ignore
			}
			this.process = null;
		}

		if (this.tempProfileDir) {
			try {
				fs.rmSync(this.tempProfileDir, { recursive: true, force: true });
			} catch {
				// ignore
			}
			this.tempProfileDir = null;
		}

		this.marionettePort = null;
		this.running = false;
	}
}
