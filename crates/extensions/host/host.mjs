import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { register } from "node:module";
import { pathToFileURL } from "node:url";

import { drain } from "./common.mjs";

register("./register.mjs", import.meta.url);

const protocol = process.stdout.write.bind(process.stdout);
process.stdout.write = (chunk, encoding, callback) => process.stderr.write(chunk, encoding, callback);

const UNSUPPORTED = [
	"registerTool",
	"registerShortcut",
	"registerFlag",
	"getFlag",
	"registerMessageRenderer",
	"registerMarkdownTransformer",
	"registerEntryRenderer",
	"registerToolRenderer",
	"sendMessage",
	"sendUserMessage",
	"appendEntry",
	"setSessionName",
	"getSessionName",
	"setLabel",
	"exec",
	"getActiveTools",
	"getAllTools",
	"getSettings",
	"setActiveTools",
	"getCommands",
	"setModel",
	"getThinkingLevel",
	"setThinkingLevel",
	"registerProvider",
	"unregisterProvider",
	"registerMcpServer",
	"unregisterMcpServer",
	"getMcpServers",
	"registerVirtualModel",
	"unregisterVirtualModel",
	"route",
];

function send(message) {
	protocol(`${JSON.stringify(message)}\n`);
}

function stripBom(text) {
	return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
}

function manifestOf(dir) {
	try {
		const pkg = JSON.parse(stripBom(fs.readFileSync(path.join(dir, "package.json"), "utf8")));
		const pi = pkg.pi;
		if (typeof pi !== "object" || pi === null) return null;
		const extensions = Array.isArray(pi.extensions) && pi.extensions.every((entry) => typeof entry === "string");
		return extensions ? { extensions: pi.extensions } : {};
	} catch {
		return null;
	}
}

function entriesOf(dir) {
	const manifest = manifestOf(dir);
	if (manifest?.extensions?.length) {
		const entries = manifest.extensions
			.map((entry) => path.resolve(dir, entry))
			.filter((entry) => fs.existsSync(entry));
		if (entries.length > 0) return entries;
	}
	for (const name of ["index.ts", "index.js"]) {
		const candidate = path.join(dir, name);
		if (fs.existsSync(candidate)) return [candidate];
	}
	return null;
}

function discover(dir) {
	let listing;
	try {
		listing = fs.readdirSync(dir, { withFileTypes: true });
	} catch {
		return [];
	}
	const found = [];
	for (const entry of listing) {
		const full = path.join(dir, entry.name);
		if ((entry.isFile() || entry.isSymbolicLink()) && (entry.name.endsWith(".ts") || entry.name.endsWith(".js"))) {
			found.push(full);
			continue;
		}
		if (entry.isDirectory() || entry.isSymbolicLink()) {
			const inner = entriesOf(full);
			if (inner) found.push(...inner);
		}
	}
	return found;
}

function resolvePaths(request) {
	const all = [];
	const seen = new Set();
	const add = (list) => {
		for (const entry of list) {
			const resolved = path.resolve(entry);
			if (seen.has(resolved)) continue;
			seen.add(resolved);
			all.push(entry);
		}
	};
	for (const dir of request.dirs ?? []) add(discover(dir));
	for (const source of request.paths ?? []) {
		const resolved = path.resolve(request.cwd, source);
		let stats;
		try {
			stats = fs.statSync(resolved);
		} catch {
			add([resolved]);
			continue;
		}
		if (stats.isDirectory()) {
			const entries = entriesOf(resolved);
			if (entries) {
				add(entries);
				continue;
			}
			add(discover(resolved));
			continue;
		}
		add([resolved]);
	}
	return all;
}

function create(extensionPath) {
	return {
		path: extensionPath,
		resolvedPath: extensionPath,
		events: [],
		commands: [],
		unsupported: [],
		eventHandlers: new Map(),
		commandHandlers: new Map(),
	};
}

function apiFor(extension) {
	const note = (member) => {
		if (!extension.unsupported.includes(member)) extension.unsupported.push(member);
	};
	const api = {
		on(name, handler) {
			extension.events.push(name);
			const handlers = extension.eventHandlers.get(name) ?? [];
			handlers.push(handler);
			extension.eventHandlers.set(name, handlers);
			return () => {
				const registered = extension.eventHandlers.get(name);
				if (!registered) return;
				const index = registered.indexOf(handler);
				if (index >= 0) registered.splice(index, 1);
			};
		},
		registerCommand(name, options) {
			extension.commands.push({ name, description: options?.description ?? "" });
			extension.commandHandlers.set(name, options?.handler);
		},
		events: {},
	};
	for (const member of UNSUPPORTED) api[member] = () => note(member);
	return api;
}

function summary(extension) {
	return {
		path: extension.path,
		resolvedPath: extension.resolvedPath,
		events: extension.events,
		commands: extension.commands,
		unsupported: extension.unsupported,
	};
}

const loaded = new Map();

async function load(request) {
	globalThis.__piExtensionAgentDir = request.agentDir;
	const extensions = [];
	const errors = [];
	for (const extensionPath of resolvePaths(request)) {
		try {
			if (loaded.has(extensionPath)) {
				extensions.push(summary(loaded.get(extensionPath)));
				continue;
			}
			const module = await import(pathToFileURL(extensionPath).href);
			const factory = module.default;
			if (typeof factory !== "function") {
				errors.push({
					path: extensionPath,
					error: `Extension does not export a valid factory function: ${extensionPath}`,
				});
				continue;
			}
			const extension = create(extensionPath);
			drain();
			await factory(apiFor(extension));
			for (const name of drain()) {
				if (!extension.unsupported.includes(name)) extension.unsupported.push(name);
			}
			loaded.set(extensionPath, extension);
			extensions.push(summary(extension));
		} catch (error) {
			errors.push({
				path: extensionPath,
				error: `Failed to load extension: ${error instanceof Error ? error.message : String(error)}`,
			});
		}
	}
	return { extensions, errors };
}

async function handle(request) {
	switch (request.op) {
		case "load":
			return await load(request);
		case "shutdown":
			process.exit(0);
		default:
			throw new Error(`unknown op: ${request.op}`);
	}
}

let queue = Promise.resolve();
const input = readline.createInterface({ input: process.stdin, terminal: false });
input.on("line", (line) => {
	queue = queue.then(async () => {
		let request;
		try {
			request = JSON.parse(line);
		} catch {
			return;
		}
		try {
			send({ id: request.id, ok: true, value: await handle(request) });
		} catch (error) {
			send({ id: request.id, ok: false, error: error instanceof Error ? error.message : String(error) });
		}
	});
});
input.on("close", () => process.exit(0));
