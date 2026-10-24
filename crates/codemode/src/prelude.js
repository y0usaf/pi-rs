(function (bridge) {
	"use strict";

	const stringify = JSON.stringify;
	let finished = false;

	function done(ok, payload, writes) {
		if (finished) return;
		finished = true;
		bridge("done", ok, payload, writes);
	}

	function serialize(value) {
		return value === undefined ? undefined : stringify(value);
	}

	function errorText(error) {
		const head = error.message ? error.name + ": " + error.message : String(error.name);
		const frames =
			typeof error.stack === "string"
				? error.stack.split("\n").filter((line) => line.trim() && !line.includes("codemode-prelude.js"))
				: [];
		return [head, ...frames].join("\n");
	}

	function format(value) {
		if (typeof value === "string") return value;
		if (value instanceof Error) return errorText(value);
		try {
			const json = stringify(value);
			return json === undefined ? String(value) : json;
		} catch {
			return String(value);
		}
	}

	function describeError(error) {
		try {
			if (error instanceof Error) {
				return stringify({ name: String(error.name), message: String(error.message), stack: errorText(error) });
			}
			return stringify({ message: String(format(error)) });
		} catch {
			return stringify({ message: "The script threw a value that cannot be described" });
		}
	}

	function output(kind, data) {
		if (finished) return;
		bridge("output", kind, data, undefined);
	}

	function outputText(value) {
		if (value === undefined || value === null || typeof value !== "object" && typeof value !== "function") {
			return String(value);
		}
		const json = stringify(value);
		return json === undefined ? String(value) : json;
	}

	function text(value) {
		let rendered;
		try {
			rendered = outputText(value);
		} catch (error) {
			throw new TypeError(error instanceof Error ? error.message : String(error));
		}
		output("text", rendered);
	}

	const console = {};
	for (const level of ["log", "info", "warn", "error", "debug"]) {
		console[level] = (...args) => {
			output("console", args.map(format).join(" "));
		};
	}
	Object.freeze(console);

	Object.defineProperty(globalThis, "console", { value: console, enumerable: true });
	Object.defineProperty(globalThis, "text", { value: text, enumerable: true });

	return {
		run(fn) {
			let promise;
			try {
				promise = fn(undefined, console);
			} catch (error) {
				done(false, describeError(error));
				return;
			}
			Promise.prototype.then.call(
				promise,
				(value) => {
					let json;
					try {
						json = serialize(value);
					} catch (error) {
						done(false, describeError(error));
						return;
					}
					done(true, json, "[]");
				},
				(error) => {
					done(false, describeError(error));
				},
			);
		},
	};
})
