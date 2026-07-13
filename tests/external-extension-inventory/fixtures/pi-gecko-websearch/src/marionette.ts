import * as net from "net";

/**
 * Minimal Marionette TCP client for Gecko browsers.
 *
 * Protocol:
 *   Messages are length-prefixed JSON. The wire format is:
 *     <ascii-decimal-length>:<json-payload>
 *
 *   The length prefix is the **byte** length of the JSON payload.
 *
 *   Request:  [0, messageId, "WebDriver:Command", params]
 *   Response: [1, messageId, errorOrNull, resultOrNull]
 */
export class MarionetteClient {
	private socket: net.Socket | null = null;
	private msgId = 0;
	private buffer = Buffer.alloc(0);
	private pendingResolve: ((msg: any) => void) | null = null;
	private pendingReject: ((err: Error) => void) | null = null;
	private connected = false;

	/** Connect to Marionette on the given port. Reads and discards the greeting. */
	async connect(port: number = 2828, host: string = "127.0.0.1", timeoutMs: number = 10000): Promise<void> {
		this.socket?.destroy();
		this.socket = null;
		this.connected = false;
		this.buffer = Buffer.alloc(0);

		return new Promise<void>((resolve, reject) => {
			const socket = net.createConnection({ port, host }, () => {
				this.connected = true;
			});
			this.socket = socket;

			// Do NOT set encoding — we need raw Buffers to handle byte-length framing correctly.

			let gotGreeting = false;
			let settled = false;
			let timer: ReturnType<typeof setTimeout> | undefined;

			const failBeforeGreeting = (err: Error) => {
				if (settled) return;
				settled = true;
				if (timer) clearTimeout(timer);
				this.connected = false;
				if (this.socket === socket) this.socket = null;
				socket.destroy();
				reject(err);
			};

			timer = setTimeout(() => {
				failBeforeGreeting(new Error(`Timed out waiting for Marionette greeting on ${host}:${port}`));
			}, timeoutMs);

			socket.on("data", (chunk: Buffer) => {
				this.buffer = Buffer.concat([this.buffer, chunk]);

				if (!gotGreeting) {
					// The greeting is the first length-prefixed message
					let parsed: any | null;
					try {
						parsed = this.tryParseMessage();
					} catch (error) {
						failBeforeGreeting(error instanceof Error ? error : new Error(String(error)));
						return;
					}
					if (parsed !== null) {
						gotGreeting = true;
						settled = true;
						if (timer) clearTimeout(timer);
						resolve();
					}
					return;
				}

				// Try to deliver a response to any pending request
				this.tryDeliverResponse();
			});

			socket.on("error", (err) => {
				if (!gotGreeting) {
					failBeforeGreeting(err);
				} else if (this.pendingReject) {
					this.pendingReject(err);
					this.pendingResolve = null;
					this.pendingReject = null;
				}
			});

			socket.on("close", () => {
				this.connected = false;
				if (!gotGreeting) {
					failBeforeGreeting(new Error("Marionette connection closed before greeting"));
				} else if (this.pendingReject) {
					this.pendingReject(new Error("Marionette connection closed"));
					this.pendingResolve = null;
					this.pendingReject = null;
				}
			});
		});
	}

	/**
	 * Try to parse one length-prefixed message from the buffer.
	 * The length prefix is in bytes, so we work with Buffer throughout.
	 * Returns the parsed JSON or null if not enough data yet.
	 */
	private tryParseMessage(): any | null {
		// Find the colon (0x3A) that separates the length prefix from the payload
		const colonIdx = this.buffer.indexOf(0x3a); // ':'
		if (colonIdx === -1) return null;

		const lengthStr = this.buffer.subarray(0, colonIdx).toString("ascii");
		const length = parseInt(lengthStr, 10);
		if (Number.isNaN(length)) return null;

		const payloadStart = colonIdx + 1;
		const payloadEnd = payloadStart + length;
		if (this.buffer.length < payloadEnd) return null;

		const payload = this.buffer.subarray(payloadStart, payloadEnd).toString("utf8");
		this.buffer = this.buffer.subarray(payloadEnd);

		return JSON.parse(payload);
	}

	/** If there's a pending promise and a full message in the buffer, resolve it. */
	private tryDeliverResponse(): void {
		if (!this.pendingResolve) return;
		const msg = this.tryParseMessage();
		if (msg !== null) {
			const res = this.pendingResolve;
			this.pendingResolve = null;
			this.pendingReject = null;
			res(msg);
		}
	}

	/**
	 * Send a Marionette command and await its response.
	 * @param command  e.g. "WebDriver:NewSession"
	 * @param params   command parameters object
	 * @param timeoutMs how long to wait for a response (default 30 s)
	 */
	async send(command: string, params: object = {}, timeoutMs: number = 30000): Promise<any> {
		if (!this.socket || !this.connected) {
			throw new Error("Marionette not connected");
		}

		const id = this.msgId++;
		const message = JSON.stringify([0, id, command, params]);
		const wire = `${Buffer.byteLength(message, "utf8")}:${message}`;

		return new Promise<any>((resolve, reject) => {
			// Set up pending handlers
			this.pendingResolve = (msg: any) => {
				clearTimeout(timer);
				// msg is [1, msgId, error, result]
				if (!Array.isArray(msg) || msg.length < 4) {
					reject(new Error(`Unexpected Marionette response: ${JSON.stringify(msg)}`));
					return;
				}
				const [, , error, result] = msg;
				if (error) {
					const errMsg = typeof error === "object" ? error.message || JSON.stringify(error) : String(error);
					reject(new Error(`Marionette error: ${errMsg}`));
					return;
				}
				resolve(result);
			};
			this.pendingReject = (err: Error) => {
				clearTimeout(timer);
				reject(err);
			};

			const timer = setTimeout(() => {
				this.pendingResolve = null;
				this.pendingReject = null;
				reject(new Error(`Marionette command '${command}' timed out after ${timeoutMs}ms`));
			}, timeoutMs);

			this.socket!.write(wire, "utf8");

			// Check if there's already a response in the buffer
			this.tryDeliverResponse();
		});
	}

	/** Create a new WebDriver session. */
	async newSession(): Promise<any> {
		return this.send("WebDriver:NewSession", {
			capabilities: {
				alwaysMatch: {
					acceptInsecureCerts: true,
				},
			},
		});
	}

	/** Navigate to a URL and wait for page load. */
	async navigate(url: string, timeoutMs: number = 30000): Promise<void> {
		await this.send("WebDriver:Navigate", { url }, timeoutMs);
	}

	/** Execute a synchronous script in the page context and return its result. */
	async executeScript(script: string, args: any[] = [], timeoutMs: number = 10000): Promise<any> {
		const result = await this.send("WebDriver:ExecuteScript", { script, args }, timeoutMs);
		return result?.value;
	}

	/** Get the full page source HTML. */
	async getPageSource(timeoutMs: number = 10000): Promise<string> {
		const result = await this.send("WebDriver:GetPageSource", {}, timeoutMs);
		return result?.value ?? "";
	}

	/** Close session and disconnect. */
	async close(): Promise<void> {
		if (!this.connected) return;
		try {
			await this.send("WebDriver:DeleteSession", {}, 5000);
		} catch {
			// Best effort
		}
		this.socket?.destroy();
		this.socket = null;
		this.connected = false;
		this.buffer = Buffer.alloc(0);
	}

	get isConnected(): boolean {
		return this.connected;
	}
}
