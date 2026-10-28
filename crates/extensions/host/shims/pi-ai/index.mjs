export { Type } from "typebox";

/** Upstream `EventStream`: events pushed until a completing one, read by async iteration. */
export class EventStream {
	queue = [];
	waiting = [];
	done = false;

	constructor(isComplete, extractResult) {
		this.isComplete = isComplete;
		this.extractResult = extractResult;
		this.finalResult = new Promise((resolve) => {
			this.resolveFinalResult = resolve;
		});
	}

	push(event) {
		if (this.done) return;
		if (this.isComplete(event)) {
			this.done = true;
			this.resolveFinalResult(this.extractResult(event));
		}
		const waiter = this.waiting.shift();
		if (waiter) waiter({ value: event, done: false });
		else this.queue.push(event);
	}

	end(result) {
		this.done = true;
		if (result !== undefined) this.resolveFinalResult(result);
		for (const waiter of this.waiting.splice(0)) waiter({ value: undefined, done: true });
	}

	async *[Symbol.asyncIterator]() {
		while (true) {
			if (this.queue.length > 0) {
				yield this.queue.shift();
			} else if (this.done) {
				return;
			} else {
				const next = await new Promise((resolve) => this.waiting.push(resolve));
				if (next.done) return;
				yield next.value;
			}
		}
	}

	result() {
		return this.finalResult;
	}
}

export class AssistantMessageEventStream extends EventStream {
	constructor() {
		super(
			(event) => event.type === "done" || event.type === "error",
			(event) => (event.type === "done" ? event.message : event.error),
		);
	}
}

export function createAssistantMessageEventStream() {
	return new AssistantMessageEventStream();
}
