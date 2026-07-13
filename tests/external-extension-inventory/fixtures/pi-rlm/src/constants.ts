// ── Defaults ────────────────────────────────────────────────────────

export const DEFAULT_MAX_DEPTH = 5; // Per-call/config <=0 = unlimited
export const DEFAULT_MAX_TURNS = 30; // Per-call/config <=0 = unlimited
export const DEFAULT_MAX_CALLS = 128; // Per-call/config <=0 = unlimited
export const DEFAULT_MAX_QUERIES = 256; // Per-call/config <=0 = unlimited
export const DEFAULT_MAX_CONCURRENT = 5; // Per-call/config <=0 = no explicit cap
export const DEFAULT_MAX_TIMEOUT_MS = 0; // 0 = unlimited
export const DEFAULT_MAX_TOKENS = 0; // 0 = unlimited
export const DEFAULT_MAX_BUDGET = 0; // USD, 0 = unlimited
export const DEFAULT_MAX_ERRORS = 0; // 0 = unlimited

export const HARD_MAX_DEPTH = 8;
export const HARD_MAX_TURNS = 80;
export const HARD_MAX_CALLS = 128;
export const HARD_MAX_QUERIES = 256;
export const HARD_MAX_CONCURRENT = 32;
export const HARD_MAX_TIMEOUT_MS = 24 * 60 * 60 * 1000;
export const HARD_MAX_TOKENS = 20_000_000;
export const HARD_MAX_BUDGET = 10_000;
export const HARD_MAX_ERRORS = 1_000;

export const MAX_RESULT_CHARS = 50_000;
export const MAX_QUERY_CONTEXT_CHARS = 500_000;
export const MAX_TRACE_TEXT_CHARS = 800;
export const MAX_INLINE_CHILD_CONTEXT_CHARS = 20_000;
export const MAX_CONTEXT_MANIFEST_CHARS = 30_000;
export const MAX_CONTEXT_TREE_ENTRIES = 500;
export const MAX_CONTEXT_TREE_DEPTH = 4;
export const MAX_CTX_OUTPUT_CHARS = 20_000;
export const DEFAULT_CTX_PEEK_CHARS = 4_000;
export const HARD_CTX_PEEK_CHARS = 20_000;
export const DEFAULT_CTX_GREP_MATCHES = 50;
export const HARD_CTX_GREP_MATCHES = 200;
export const MAX_CTX_GREP_FILES = 5_000;

export const REPL_TOOL_NAME = "repl";
export const RLM_FINAL_OUTPUT_CUSTOM_TYPE = "rlm_final";

export const RLM_CALLS = ["llm_query", "llm_query_batched", "rlm_query", "rlm_query_batched"] as const;
export type RlmCall = typeof RLM_CALLS[number];
export type ExecutionKind = "llm" | "rlm";

export const CONTEXT_MODES = ["auto", "inline", "file_backed"] as const;
export type ContextMode = typeof CONTEXT_MODES[number];

export type ContextSourceKind = "inline" | "file" | "dir" | "missing" | "other";

export interface ContextSource {
  id: string;
  name?: string;
  label: string;
  input?: string;
  path: string;
  relPath: string;
  kind: ContextSourceKind;
  sizeBytes?: number;
  entries?: number;
  error?: string;
}

export interface ContextStore {
  dir: string;
  scratchDir: string;
  notesDir: string;
  artifactsDir: string;
  manifestPath: string;
  manifestJsonPath: string;
  readmePath: string;
  manifestText: string;
  sources: ContextSource[];
}

export interface Budget {
  calls: number;
  maxCalls: number | undefined;
  queries: number;
  maxQueries: number | undefined;
  tokens: number;
  maxTokens: number; // 0 = unlimited
  cost: number;
  maxBudget: number; // USD, 0 = unlimited
  errors: number;
  maxErrors: number; // 0 = unlimited
  startTimeMs: number;
  maxTimeoutMs: number; // 0 = unlimited
}

export interface RunState {
  runId: string;
  maxDepth: number | undefined;
  maxConcurrent: number | undefined;
  maxTurns: number | undefined;
  budget: Budget;
  /** The model of the parent Pi session that started this RLM run. */
  model?: any;

  logPath?: string;
}

export interface BatchItem {
  prompt: string;
  rootPrompt?: string;
  model?: string;

  context?: string;
  contextMode?: ContextMode;
  paths?: string[];
  sources?: Array<{ name?: string; path: string }>;
  contextName?: string;
}

export interface Details {
  call: RlmCall;
  kind: ExecutionKind;
  depth: number;
  maxDepth: number | undefined;
  callsUsed: number;
  maxCalls: number | undefined;
  queriesUsed: number;
  maxQueries: number | undefined;
  turns: number;
  maxTurns: number | undefined;
  model: string;
  status?: "completed" | "partial" | "error" | "aborted" | "budget_exhausted";
  tokensUsed?: number;
  maxTokens?: number;
  costUsed?: number;
  maxBudget?: number;
  errorsUsed?: number;
  maxErrors?: number;
  elapsedMs?: number;
  maxTimeoutMs?: number;
  prompt: string;
  rootPrompt?: string;

  usage?: { input: number; output: number; cacheRead: number; cacheWrite: number; totalTokens: number; cost: number };
  paths: string[];
  sources?: Array<{ name?: string; path: string }>;
  contextMode?: ContextMode;
  scratchDir?: string;
  contextSources?: string[];
  answer?: string;
  trace?: Array<{ role: string; toolName?: string; text: string }>;
  completedWithReturn?: boolean;
  finalMirrored?: boolean;
  finalizationRequested?: boolean;
  deterministicFinalized?: boolean;
  deterministicFinalizationReason?: string;
  abortedByTurnLimit?: boolean;
  incomplete?: boolean;
  error?: string;
  batch?: boolean;
  batchSize?: number;
  maxConcurrent?: number;
  results?: Details[];
}