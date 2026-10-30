import { AsyncLocalStorage } from "node:async_hooks";
import fs from "node:fs";

type Fn = (...args: unknown[]) => unknown;
type Bag = Record<PropertyKey, unknown>;
type Scope = { api?: boolean; tick?: { timer: string; tick: number } };

const file = process.env.PI_PARITY_LOG;
const quiet = new Set(["then", "toJSON", "constructor"]);
let next = 0;
const callbacks = new WeakMap<Fn, Map<string, Fn>>();
const watched = new WeakMap<object, object>();
const methods = new WeakMap<object, Map<string, { original: Fn; wrapped: Fn }>>();
const registrations = new Map<string, number>();
const timers = new Map<string, number>();
const scope = new AsyncLocalStorage<Scope>();
const interval = globalThis.setInterval;

function write(entry: Bag): void {
	const tick = scope.getStore()?.tick;
	if (file) fs.appendFileSync(file, `${JSON.stringify(tick ? { ...entry, ...tick } : entry)}\n`);
}

globalThis.setInterval = ((handler: Fn, delay?: number, ...rest: unknown[]) => {
	const store = scope.getStore();
	if (!store || store.api) return interval(handler, delay, ...rest);
	const base = `setInterval(${delay})`;
	const count = (timers.get(base) ?? 0) + 1;
	timers.set(base, count);
	const timer = count === 1 ? base : `${base}@${count}`;
	let tick = 0;
	return interval(
		function (this: unknown, ...args: unknown[]) {
			tick += 1;
			return scope.run({ tick: { timer, tick } }, () => {
				const id = next++;
				write({ id, kind: "callback", path: timer, args: plain(args) });
				return settle(id, () => handler.apply(this, args));
			});
		},
		delay,
		...rest,
	);
}) as typeof setInterval;

function isCtx(value: unknown): value is Bag {
	return typeof value === "object" && value !== null && "ui" in value && "sessionManager" in value;
}

function plain(value: unknown): unknown {
	const open = new Set<object>();
	const walk = (item: unknown): unknown => {
		if (item === undefined) return "<undefined>";
		if (typeof item === "function") return "<function>";
		if (typeof item === "bigint") return `${item}n`;
		if (typeof item === "symbol") return item.toString();
		if (item === null || typeof item !== "object") return item;
		if (isCtx(item)) return "<ctx>";
		if (open.has(item)) return "<cycle>";
		open.add(item);
		let out: unknown;
		if (item instanceof Error) out = { error: item.name, message: item.message };
		else if (Array.isArray(item)) out = item.map(walk);
		else if (item instanceof Map) out = { "<map>": [...item].map(walk) };
		else if (item instanceof Set) out = { "<set>": [...item].map(walk) };
		else {
			const fields: Bag = {};
			const proto = Object.getPrototypeOf(item);
			if (proto !== null && proto !== Object.prototype) fields["<class>"] = proto.constructor?.name ?? "?";
			for (const key of Object.keys(item)) fields[key] = walk((item as Bag)[key]);
			out = fields;
		}
		open.delete(item);
		return out;
	};
	return walk(value);
}

function watchable(value: unknown): value is object {
	if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
	const proto = Object.getPrototypeOf(value);
	if (proto !== null && proto !== Object.prototype) return true;
	return Object.values(value).some((field) => typeof field === "function");
}

function settle(id: number, run: () => unknown): unknown {
	let result: unknown;
	try {
		result = run();
	} catch (error) {
		write({ id, kind: "throw", value: plain(error) });
		throw error;
	}
	if (result instanceof Promise) {
		return result.then(
			(value) => {
				write({ id, kind: "resolve", value: plain(value) });
				return value;
			},
			(error) => {
				write({ id, kind: "reject", value: plain(error) });
				throw error;
			},
		);
	}
	write({ id, kind: "return", value: plain(result) });
	return result;
}

function isPlain(value: unknown): value is Bag | unknown[] {
	if (typeof value !== "object" || value === null) return false;
	if (Array.isArray(value)) return true;
	const proto = Object.getPrototypeOf(value);
	return proto === Object.prototype || proto === null;
}

function label(path: string, args: unknown[]): string {
	const [first] = args;
	if (typeof first === "string") return `${path}(${first})`;
	if (isPlain(first) && !Array.isArray(first) && typeof first.name === "string") return `${path}(${first.name})`;
	return path;
}

function bears(value: unknown, open: Set<object>): boolean {
	if (typeof value === "function") return true;
	if (!isPlain(value) || open.has(value)) return false;
	open.add(value);
	const found = Object.values(value).some((field) => bears(field, open));
	open.delete(value);
	return found;
}

function inbound(path: string, arg: unknown, open = new Set<object>()): unknown {
	if (typeof arg === "function") return callback(path, arg as Fn);
	if (!isPlain(arg) || open.has(arg) || !bears(arg, new Set())) return arg;
	open.add(arg);
	const copy = Array.isArray(arg)
		? arg.map((item, index) => inbound(`${path}[${index}]`, item, open))
		: Object.fromEntries(Object.entries(arg).map(([key, field]) => [key, inbound(`${path}.${key}`, field, open)]));
	open.delete(arg);
	return copy;
}

function outbound(path: string, arg: unknown): unknown {
	if (isCtx(arg)) return watch(arg, "ctx");
	if (typeof arg === "function") return host(path, arg as Fn, undefined);
	return arg;
}

function callback(path: string, fn: Fn): Fn {
	let paths = callbacks.get(fn);
	if (!paths) {
		paths = new Map();
		callbacks.set(fn, paths);
	}
	const known = paths.get(path);
	if (known) return known;
	const wrapped = function (this: unknown, ...args: unknown[]) {
		const id = next++;
		write({ id, kind: "callback", path, args: plain(args) });
		return scope.run({ tick: scope.getStore()?.tick }, () =>
			settle(id, () =>
				fn.apply(
					this,
					args.map((arg, index) => outbound(`${path}#${index}`, arg)),
				),
			),
		);
	};
	paths.set(path, wrapped);
	return wrapped;
}

function registration(name: string, args: unknown[]): string {
	if (!bears(args, new Set())) return name;
	const count = (registrations.get(name) ?? 0) + 1;
	registrations.set(name, count);
	return count === 1 ? name : `${name}@${count}`;
}

function host(path: string, fn: Fn, self: unknown): Fn {
	return (...args: unknown[]) => {
		const id = next++;
		const name = registration(label(path, args), args);
		write({ id, kind: "call", path: name, args: plain(args) });
		const result = scope.run({ api: true, tick: scope.getStore()?.tick }, () =>
			settle(id, () =>
				fn.apply(
					self,
					args.map((arg, index) => inbound(`${name}#${index}`, arg)),
				),
			),
		);
		return result instanceof Promise
			? result.then((value) => outbound(`${name}#return`, value))
			: outbound(`${name}#return`, result);
	};
}

function method(object: object, name: string, fn: Fn): Fn {
	let known = methods.get(object);
	if (!known) {
		known = new Map();
		methods.set(object, known);
	}
	const cached = known.get(name);
	if (cached?.original === fn) return cached.wrapped;
	const wrapped = host(name, fn, object);
	known.set(name, { original: fn, wrapped });
	return wrapped;
}

function watch<T extends object>(target: T, path: string): T {
	const known = watched.get(target);
	if (known) return known as T;
	const proxy = new Proxy(target, {
		get(object, key) {
			const value = Reflect.get(object, key, object);
			if (typeof key === "symbol") return value;
			const name = `${path}.${key}`;
			if (typeof value === "function") return method(object, name, value as Fn);
			if (quiet.has(key) && value === undefined) return value;
			if (watchable(value)) {
				write({ id: next++, kind: "get", path: name, value: "<object>" });
				return watch(value, name);
			}
			write({ id: next++, kind: "get", path: name, value: plain(value) });
			return value;
		},
	});
	watched.set(target, proxy);
	return proxy;
}

export function wrap(factory: unknown): unknown {
	if (typeof factory !== "function") return factory;
	return async (pi: object) => {
		registrations.clear();
		timers.clear();
		const id = next++;
		write({ id, kind: "load" });
		await scope.run({}, () => settle(id, () => (factory as Fn)(watch(pi, "pi"))));
	};
}
