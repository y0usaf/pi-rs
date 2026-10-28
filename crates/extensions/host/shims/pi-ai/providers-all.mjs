import { createAssistantMessageEventStream } from "./index.mjs";

const host = () => globalThis.__piHost;

function failure(model, error) {
	return {
		role: "assistant",
		content: [],
		api: model.api,
		provider: model.provider,
		model: model.id,
		usage: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		stopReason: "error",
		errorMessage: error instanceof Error ? error.message : String(error),
		timestamp: Date.now(),
	};
}

/** A built-in provider's request, which pi-rs streams with its own client for the model's API. */
function stream(model, context, options) {
	const out = createAssistantMessageEventStream();
	const { signal, fetch, onPayload, onResponse, onProviderStreamEvent, env, ...rest } = options ?? {};
	const { callId, answer } = host().askStream("stream", { model, context, options: rest }, (event) => out.push(event));
	const stop = () => host().effect({ call: "cancel", ask: callId });
	if (signal?.aborted) stop();
	else signal?.addEventListener("abort", stop, { once: true });
	answer
		.then(
			() => out.end(),
			(error) => {
				out.push({ type: "error", reason: "error", error: failure(model, error) });
				out.end();
			},
		)
		.finally(() => signal?.removeEventListener("abort", stop));
	return out;
}

/** pi-ai's built-in providers, each streaming through pi-rs. */
export function builtinProviders() {
	return host()
		.providers()
		.map((info) => ({
			id: info.id,
			name: info.name,
			baseUrl: info.baseUrl,
			auth: info.oauth ? { oauth: { ...info.oauth } } : { apiKey: { name: `${info.name} API key` } },
			getModels: () => host()
				.models()
				.filter((model) => model.provider === info.id),
			stream,
			streamSimple: stream,
		}));
}
