export type CacheType = 'f16' | 'q8_0' | 'q4_0';

export interface RuntimeConfig {
  contextLength: number;
  gpuLayers: number;
  cpuThreads: number;
  batchSize: number;
  microBatchSize: number;
  flashAttention: boolean;
  cacheTypeK: CacheType;
  cacheTypeV: CacheType;
  offloadKvCache: boolean;
  mmap: boolean;
}

export const defaultRuntimeConfig: RuntimeConfig = {
  contextLength: 131072, gpuLayers: -1, cpuThreads: 6, batchSize: 512,
  microBatchSize: 128, flashAttention: true, cacheTypeK: 'q8_0', cacheTypeV: 'q8_0',
  offloadKvCache: true, mmap: true,
};

export interface Conversation {
  id: string; title: string; updatedAt: number;
  providerId: string | null; modelId: string | null; providerSelectionRequired: boolean;
}
export interface Message {
  error?: string | null;
  id: string; conversationId: string; role: 'user' | 'assistant' | 'tool';
  content: string; reasoning: string; status: 'complete' | 'streaming' | 'interrupted' | 'error'; createdAt: number;
}
export interface Preferences {
  runtimePath: string; modelPath: string; temperature: number; topP: number; maxTokens: number;
  systemPrompt: string;
}
export interface RuntimeStatus { phase: 'stopped' | 'loading' | 'ready' | 'error'; message: string; modelPath: string | null; loadedConfig?: RuntimeConfig | null; gpuOffload?: { layers: number; totalLayers: number } | null }
export type ToolSupport = 'unknown' | 'supported' | 'unsupported';
export interface RemoteModel { id: string; contextLength: number | null; maxOutputTokens: number | null; toolSupport: ToolSupport }
export interface ProviderConnection {
  id: string; name: string; apiFormat: 'openai-chat-completions' | 'anthropic-messages'; baseUrl: string;
  verified: boolean; lastTestedAt: number | null; models: RemoteModel[]; hasApiKey: boolean;
}
export interface ProviderDraft {
  id?: string; name: string; apiFormat: 'openai-chat-completions' | 'anthropic-messages'; baseUrl: string;
  apiKey?: string; models: RemoteModel[];
}
export interface ProviderTestResult { verified: boolean; modelListSupported: boolean; models: string[]; message: string }
export interface ModelSelection { providerId: string | null; modelId: string }
export interface Bootstrap {
  conversations: Conversation[]; config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus;
  rememberedTools: RememberedTools; providers: ProviderConnection[]; preferredModel: ModelSelection;
}
export interface RememberedTools { sources: string[]; tools: ToolSelection[] }
export interface ToolApproval { id: string; connector: string; localServerName?: string | null; name: string; arguments: Record<string, unknown> }
export interface ContextUsage { inputTokens: number; responseReserve: number; contextLength: number; estimated?: boolean }
export type RunState =
  | 'preparing'
  | 'generating'
  | 'awaiting_approval'
  | 'executing_tools'
  | 'preparing_next_round'
  | 'completed'
  | 'cancelled'
  | 'failed'
  | 'outcome_unknown';

export interface RunRecord {
  id: string;
  conversationId: string;
  status: RunState;
  modelProvider?: string | null;
  modelId?: string | null;
  checkpoint?: string | null;
  error?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface RunEvent {
  runId: string;
  seq: number;
  stepId: string;
  toolCallId?: string | null;
  eventType: string;
  payload: Record<string, unknown>;
  createdAt: number;
}

export interface ArtifactRecord {
  id: string;
  conversationId: string;
  runId?: string | null;
  toolName: string;
  mimeType: string;
  sizeBytes: number;
  sha256: string;
  content: string;
  createdAt: number;
}

export interface ChatEvent {
  messageId: string;
  content: string;
  reasoning: string;
  approval?: ToolApproval | null;
  context?: ContextUsage;
  runId?: string;
  stepId?: string;
  seq?: number;
  state?: RunState;
  activity?: string;
}
export interface ToolView { name: string; description: string; inputSchema: Record<string, unknown> }
export interface ToolSelection { connectorId: string; toolName: string }
export interface ConnectorView {
  id: string; description: string; url: string; authType: 'none' | 'apiKey' | 'oauth' | 'local';
  connected: boolean; hasCredential: boolean; tools: ToolView[];
}
export interface SkillView {
  id: string; description: string; repo: string; revision: string; sourcePath: string;
  files: { path: string; size: number; sha256: string }[]; installed: boolean; active: boolean;
}
export interface SkillDependencyStatus {
  dependency: { kind: string; name: string; detail: string };
  satisfied: boolean; remedy: string;
}
export interface SkillUpdateStatus {
  id: string; installedRevision: string | null; catalogRevision: string;
  updateAvailable: boolean; intact: boolean; problem: string | null;
}
export interface ExecutionConfig { pythonPath: string; nodePath: string; powershellPath: string }
export interface HardwareStatus {
  logicalCpus: number; memoryTotalBytes: number | null; memoryAvailableBytes: number | null;
  gpuStatus: string; sampledAt: number;
  gpus: { name: string; uuid: string; memoryUsedMib: number | null; memoryTotalMib: number | null; utilizationPercent: number | null; driverVersion: string }[];
}
export type AccessMode = 'ask' | 'autoApprove' | 'fullAccess';
export interface ConversationTools { sources: string[]; tools: ToolSelection[]; accessMode: AccessMode }
export type CapabilityKind = 'Model' | 'Tool' | 'Skill' | 'Session' | 'Sandbox' | 'Storage' | 'Loop' | 'Scheduling' | 'Ui';
export interface Capability { id: string; kind: CapabilityKind | string; version: number; enabled: boolean; description: string; config?: Record<string, unknown> }
export interface SessionEvent {
  id: string;
  conversationId: string;
  runId?: string | null;
  seq: number;
  stepId?: string | null;
  toolCallId?: string | null;
  eventType: string;
  payload: Record<string, unknown> | unknown;
  ignorable: boolean;
  createdAt: number;
}
export interface ForkSessionResult {
  newConversation: Conversation;
  copiedEventsCount: number;
}
export interface Preset { id: string; name: string; description: string; sources: string[]; mcp: boolean; systemTime: boolean; skills: boolean; harness: string[] }
export interface Todo { text: string; status: 'pending' | 'in_progress' | 'completed'; updatedAt: number }
export interface SubagentRun {
  id: string; conversationId: string; parentRunId: string; childRunId: string;
  depth: number; status: string; label: string; prompt: string; error?: string | null;
  createdAt: number; updatedAt: number;
}
export interface Fact { id: string; scope: string; fact: string; origin: string; createdAt: number; updatedAt: number }
export interface Schedule {
  id: string; name: string; cron: string; task: string; conversationId?: string | null;
  allowWrite: boolean; enabled: boolean; runOnce: boolean;
  lastRunAt: number | null; lastResult: string | null; createdAt: number;
}
export interface Plugin { name: string; version: string; path: string; enabled: boolean; sha256: string; installedAt: number }
export interface ScanVerdict { verdict: 'install' | 'caution' | 'reject'; findings: string[] }
export interface ProviderFormat { id: string; defaultBaseUrl: string | null }
