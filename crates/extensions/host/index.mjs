import * as upstream from "./upstream.js";

export * from "./upstream.js";

const ENGINE = [
	"AgentSession",
	"AgentSessionRuntime",
	"DefaultPackageManager",
	"DefaultResourceLoader",
	"InteractiveMode",
	"ModelRegistry",
	"ModelRuntime",
	"ProjectTrustStore",
	"SessionManager",
	"SettingsManager",
	"createAgentSession",
	"createAgentSessionFromServices",
	"createAgentSessionRuntime",
	"createAgentSessionServices",
	"main",
	"readStoredCredential",
	"runPrintMode",
	"runRpcMode",
];

function unported(member) {
	return globalThis.__piHost?.unported(member) ?? new Error(`pi-rs does not implement ${member} yet`);
}

function standIn(name) {
	const real = upstream[name];
	const throwing = real.constructor.name === "AsyncFunction"
		? async () => {
				throw unported(name);
			}
		: () => {
				throw unported(name);
			};
	if (!/^class\b/.test(Function.prototype.toString.call(real))) return throwing;
	const Class = class {
		constructor() {
			throw unported(name);
		}
	};
	Object.defineProperty(Class, "name", { value: name });
	for (const key of Object.getOwnPropertyNames(real)) {
		if (key === "prototype" || key === "name" || key === "length" || typeof real[key] !== "function") continue;
		Class[key] = () => {
			throw unported(`${name}.${key}`);
		};
	}
	return Class;
}

const standIns = Object.fromEntries(ENGINE.map((name) => [name, standIn(name)]));

/** Upstream's engine, which pi-rs runs in Rust: each stand-in throws until pi-rs backs it. */
export const {
	AgentSession,
	AgentSessionRuntime,
	DefaultPackageManager,
	DefaultResourceLoader,
	InteractiveMode,
	ModelRegistry,
	ModelRuntime,
	ProjectTrustStore,
	SessionManager,
	SettingsManager,
	createAgentSession,
	createAgentSessionFromServices,
	createAgentSessionRuntime,
	createAgentSessionServices,
	main,
	readStoredCredential,
	runPrintMode,
	runRpcMode,
} = standIns;
