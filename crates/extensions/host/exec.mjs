import { spawn } from "node:child_process";

const groups = new Set();
let pruning;

function effect(call) {
	globalThis.__piHost?.effect(call);
}

function signalGroup(child, name) {
	if (child.pid === undefined) return;
	try {
		process.kill(-child.pid, name);
	} catch (error) {
		if (error.code !== "ESRCH") throw error;
	}
}

function alive(child) {
	try {
		process.kill(-child.pid, 0);
		return true;
	} catch (error) {
		if (error.code === "ESRCH" || error.code === "EPERM") return false;
		throw error;
	}
}

function prune() {
	for (const child of groups) {
		if (alive(child)) continue;
		groups.delete(child);
		effect({ call: "ungroup", pid: child.pid });
	}
	if (groups.size > 0) return;
	clearInterval(pruning);
	pruning = undefined;
}

function track(child) {
	if (child.pid === undefined) return;
	groups.add(child);
	effect({ call: "group", pid: child.pid });
	pruning ??= setInterval(prune, 10000).unref();
}

export function killAll() {
	prune();
	for (const child of groups) signalGroup(child, "SIGKILL");
}

function waitForChild(child, stopped) {
	return new Promise((resolve, reject) => {
		let settled = false;
		let exited = false;
		let exitCode = null;
		let timer;
		let stdoutEnded = child.stdout === null;
		let stderrEnded = child.stderr === null;
		const finish = (code) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			child.stdout?.destroy();
			child.stderr?.destroy();
			resolve(code);
		};
		const afterExit = () => {
			if (exited && stdoutEnded && stderrEnded) finish(exitCode);
		};
		const idle = () => {
			clearTimeout(timer);
			timer = setTimeout(() => finish(exitCode), 100);
		};
		const data = () => {
			if (exited && !settled && !stopped()) idle();
		};
		child.stdout?.once("end", () => {
			stdoutEnded = true;
			afterExit();
		});
		child.stderr?.once("end", () => {
			stderrEnded = true;
			afterExit();
		});
		child.stdout?.on("data", data);
		child.stderr?.on("data", data);
		child.once("error", (error) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			reject(error);
		});
		child.once("exit", (code) => {
			exited = true;
			exitCode = code;
			afterExit();
			if (!settled) idle();
		});
		child.once("close", finish);
	});
}

/** Upstream's `execCommand`, run in its own process group so a timeout, an abort or pi-rs stopping ends every process it started. */
export function execCommand(command, args, cwd, options) {
	return new Promise((resolve) => {
		const proc = spawn(command, args, { cwd, detached: true, shell: false, stdio: ["ignore", "pipe", "pipe"] });
		track(proc);
		let stdout = "";
		let stderr = "";
		let killed = false;
		let timeoutId;
		const kill = () => {
			if (killed) return;
			killed = true;
			signalGroup(proc, "SIGTERM");
			setTimeout(() => {
				signalGroup(proc, "SIGKILL");
				prune();
			}, 5000).unref();
		};
		if (options?.signal) {
			if (options.signal.aborted) kill();
			else options.signal.addEventListener("abort", kill, { once: true });
		}
		if (options?.timeout && options.timeout > 0) timeoutId = setTimeout(kill, options.timeout);
		proc.stdout?.on("data", (chunk) => {
			stdout += chunk.toString();
		});
		proc.stderr?.on("data", (chunk) => {
			stderr += chunk.toString();
		});
		const done = (code) => {
			prune();
			clearTimeout(timeoutId);
			options?.signal?.removeEventListener("abort", kill);
			resolve({ stdout, stderr, code, killed });
		};
		waitForChild(proc, () => killed).then(
			(code) => done(code ?? 0),
			() => done(1),
		);
	});
}
