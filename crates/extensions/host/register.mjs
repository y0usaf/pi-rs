const hostModules = new URL("./node_modules/", import.meta.url);

const missing = new Set(["ERR_MODULE_NOT_FOUND", "MODULE_NOT_FOUND", "ERR_UNSUPPORTED_DIR_IMPORT"]);

const isBare = (specifier) => !specifier.startsWith(".") && !specifier.startsWith("/") && !specifier.includes(":");

export async function resolve(specifier, context, next) {
	try {
		return await next(specifier, context);
	} catch (error) {
		if (!missing.has(error?.code)) throw error;
		if (isBare(specifier)) return next(specifier, { ...context, parentURL: hostModules.href });
		if (specifier.endsWith(".js")) return next(`${specifier.slice(0, -3)}.ts`, context);
		throw error;
	}
}

const declares = (source, name) => new RegExp(`\\b(?:const|let|var|function|class|as)\\s+${name}\\b`).test(source);

const CJS_GLOBALS = [
	[/\b__filename\b/, "__filename", "const __filename = import.meta.filename;"],
	[/\b__dirname\b/, "__dirname", "const __dirname = import.meta.dirname;"],
	[
		/\brequire\s*[(.]/,
		"require",
		'import { createRequire as __piCreateRequire } from "node:module"; const require = __piCreateRequire(import.meta.url);',
	],
];

export async function load(url, context, next) {
	const loaded = await next(url, context);
	if (!url.startsWith("file:") || url.includes("/node_modules/") || !loaded.format?.startsWith("module")) {
		return loaded;
	}
	const source = typeof loaded.source === "string" ? loaded.source : new TextDecoder().decode(loaded.source);
	const prelude = CJS_GLOBALS.filter(([used, name]) => used.test(source) && !declares(source, name))
		.map(([, , line]) => line)
		.join(" ");
	if (!prelude) return loaded;
	const start = source.startsWith("#!") ? source.indexOf("\n") + 1 : 0;
	return { ...loaded, source: source.slice(0, start) + prelude + " " + source.slice(start) };
}
