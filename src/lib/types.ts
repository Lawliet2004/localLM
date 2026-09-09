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
  contextLength: 8192, gpuLayers: -1, cpuThreads: 6, batchSize: 512,
  microBatchSize: 128, flashAttention: true, cacheTypeK: 'q8_0', cacheTypeV: 'q8_0',
  offloadKvCache: true, mmap: true,
};

export interface Conversation { id: string; title: string; updatedAt: number }
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
export interface Bootstrap { conversations: Conversation[]; config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus }
export interface ToolApproval { id: string; connector: string; localServerName?: string | null; name: string; arguments: Record<string, unknown> }
export interface ContextUsage { inputTokens: number; responseReserve: number; contextLength: number }
export interface ChatEvent { messageId: string; content: string; reasoning: string; approval?: ToolApproval | null; context?: ContextUsage }
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
export interface ExecutionConfig { pythonPath: string; nodePath: string; powershellPath: string }
export interface HardwareStatus {
  logicalCpus: number; memoryTotalBytes: number | null; memoryAvailableBytes: number | null;
  gpuStatus: string; sampledAt: number;
  gpus: { name: string; uuid: string; memoryUsedMib: number | null; memoryTotalMib: number | null; utilizationPercent: number | null; driverVersion: string }[];
}
export type AccessMode = 'ask' | 'autoApprove' | 'fullAccess';
export interface ConversationTools { sources: string[]; tools: ToolSelection[]; accessMode: AccessMode }
