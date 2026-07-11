export const MODELS = {
	"z-provider": {
		"z-model": {
			id: "z-model",
			name: "Z Model",
			api: "anthropic-messages",
			provider: "z-provider",
			baseUrl: "https://z.invalid",
			reasoning: true,
			thinkingLevelMap: { minimal: null, low: "low" },
			input: ["text", "image"],
			cost: { input: 1, output: 2, cacheRead: 0.1, cacheWrite: 0.2 },
			contextWindow: 1000,
			maxTokens: 100,
		},
	},
	"a-provider": {
		"a-model": {
			id: "a-model",
			name: "Original Name",
			api: "openai-completions",
			provider: "a-provider",
			baseUrl: "https://a.invalid/v1",
			reasoning: false,
			input: ["text"],
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
			contextWindow: 2000,
			maxTokens: 200,
			compat: { supportsStore: false },
		},
	},
} as const;
