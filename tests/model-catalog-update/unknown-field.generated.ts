export const MODELS = {
	provider: {
		model: {
			id: "model", name: "Model", api: "anthropic-messages", provider: "provider",
			baseUrl: "https://example.invalid", reasoning: false, input: ["text"],
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
			contextWindow: 1000, maxTokens: 100, newUpstreamField: true,
		},
	},
};
