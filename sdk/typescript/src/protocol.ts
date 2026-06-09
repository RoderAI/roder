/**
 * Hand-written mirrors of the Rust wire structs. The Rust side is canonical;
 * update these when the protocol changes:
 * - crates/roder-protocol/src/lib.rs: `Thread`, `ThreadStatus`, `Turn`, `Item`,
 *   `ThreadItemEvent`/`ThreadItemEventKind`/`ThreadItemDelta`,
 *   `ExternalToolCall` and the `*Notification` param structs (camelCase wire
 *   format).
 * - crates/roder-api/src/inference.rs `TokenUsage` and
 *   crates/roder-api/src/thread.rs `ThreadUsageMetadata` (snake_case wire
 *   format, shared field names).
 */

/** Wire format of roder-api `TokenUsage` / `ThreadUsageMetadata` (snake_case). */
export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  /** Subset of `prompt_tokens` read from the provider prompt cache. */
  cached_prompt_tokens?: number;
  /** Subset of `prompt_tokens` written to the provider prompt cache. */
  cache_creation_prompt_tokens?: number;
  cache_hit_rate?: number | null;
}

export interface ThreadStatus {
  /** "idle" | "running" (open set on the wire). */
  type: string;
  activeTurnId: string | null;
  activeFlags: string[];
}

/** Mirrors roder-api ToolSpec: a host-executed tool advertised to the model. */
export interface ToolSpec {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export interface Thread {
  id: string;
  preview: string;
  modelProvider: string;
  model: string;
  selectionMode?: string;
  createdAt: number;
  updatedAt: number;
  status: ThreadStatus;
  cwd: string;
  workspaceId?: string;
  rootId?: string;
  name?: string;
  messageCount?: number;
  turns?: Turn[];
  usage?: TokenUsage;
  toolAllowlist?: string[];
  developerInstructions?: string;
  externalTools?: ToolSpec[];
}

/** Server emits `{message}` for failed turns (roder-app-server notifications.rs). */
export interface TurnError {
  message?: string;
}

export interface Turn {
  id: string;
  items: ThreadItem[];
  itemsView: string;
  /** "inProgress" | "completed" | "failed" | "interrupted" (open set on the wire). */
  status: string;
  error?: TurnError;
  startedAt?: number;
  completedAt?: number;
  durationMs?: number;
  usage?: TokenUsage;
  /**
   * Normalized stop reason of the turn's terminal inference step ("stop",
   * "length", "toolUse", "contentFilter", "refusal", or a provider-native
   * value passed through). Present on `turn/completed` for completed turns.
   */
  finishReason?: string;
}

export type ThreadItemStatus = "inProgress" | "completed" | "failed";

export interface UserMessageItem {
  type: "userMessage";
  id: string;
  text: string;
  images?: unknown[];
  status?: ThreadItemStatus;
}

export interface AgentMessageItem {
  type: "agentMessage";
  id: string;
  text: string;
  phase?: string;
  status?: ThreadItemStatus;
}

export interface ReasoningItem {
  type: "reasoning";
  id: string;
  summary?: string[];
  content?: string[];
  status?: ThreadItemStatus;
}

export interface ToolExecutionItem {
  type: "toolExecution";
  id: string;
  toolCallId: string;
  toolName: string;
  status: ThreadItemStatus;
  input?: unknown;
  output?: string;
  error?: string;
}

export interface CompactionItem {
  type: "compaction";
  id: string;
  summary: string;
  status?: ThreadItemStatus;
}

export interface ErrorItem {
  type: "error";
  id: string;
  message: string;
  status?: ThreadItemStatus;
}

export interface RawItem {
  type: "raw";
  id: string;
  payload: unknown;
  status?: ThreadItemStatus;
}

/** Mirrors roder-protocol `Item` (tagged by `type`). */
export type ThreadItem =
  | UserMessageItem
  | AgentMessageItem
  | ReasoningItem
  | ToolExecutionItem
  | CompactionItem
  | ErrorItem
  | RawItem;

/** Mirrors roder-protocol `ThreadItemDelta` (tagged by `type`). */
export type ThreadItemDelta =
  | { type: "agentMessageText"; delta: string; phase?: string }
  | { type: "reasoningText"; delta: string; contentIndex: number }
  | { type: "reasoningSummaryPartAdded"; summaryIndex: number }
  | { type: "reasoningSummaryText"; delta: string; summaryIndex: number };

/** Mirrors roder-protocol `ExternalToolCall`. */
export interface ExternalToolCall {
  id: string;
  name: string;
  /** Parsed JSON arguments from the model. */
  arguments: unknown;
}
