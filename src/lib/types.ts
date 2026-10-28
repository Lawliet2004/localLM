export type CacheType = 'f16' | 'q8_0' | 'q4_0';

export interface RuntimeConfig {
  mtpMode?: 'auto' | 'off';
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
  visionEnabled: boolean;
  offloadProjector: boolean;
  inferenceSlots: number;
}

export const defaultRuntimeConfig: RuntimeConfig = {
  mtpMode: 'auto', contextLength: 131072, gpuLayers: -1, cpuThreads: 6, batchSize: 512,
  microBatchSize: 128, flashAttention: true, cacheTypeK: 'q8_0', cacheTypeV: 'q8_0',
  offloadKvCache: true, mmap: true, inferenceSlots: 1, visionEnabled: true, offloadProjector: true,
};

export interface Conversation {
  id: string; title: string; updatedAt: number;
  providerId: string | null; modelId: string | null; providerSelectionRequired: boolean;
}
export type PresentationMode = 'auto' | 'text' | 'visual';
export interface PresentationRecord {
  messageId: string; format: 'markdown' | 'openui'; formatVersion: number;
  libraryVersion: string; sourceText: string; uiProgram: string;
  interactiveState: Record<string, unknown>;
  generationStatus: 'streaming' | 'complete' | 'interrupted' | 'error';
  diagnostic?: string | null;
}
export interface PresentationModes { defaultMode: PresentationMode; overrideMode: PresentationMode | null; effectiveMode: PresentationMode }
export interface Message {
  /** Transient round snapshots; persisted messages and the journal remain authoritative. */
  liveRounds?: LiveModelRound[];
  presentation?: PresentationRecord;
  metrics?: ResponseMetrics | null;
  error?: string | null;
  id: string; conversationId: string; role: 'user' | 'assistant' | 'tool';
  content: string; reasoning: string; status: 'complete' | 'streaming' | 'interrupted' | 'error' | 'paused'; createdAt: number;
}
export interface ToolIdentity {
  conversationId: string;
  runId?: string | null;
  stepId?: string | null;
  toolCallId: string;
}
export interface LiveModelRound {
  conversationId: string;
  runId: string;
  stepId: string;
  roundIndex: number;
  content: string;
  reasoning: string;
  lastSeq: number;
  createdAt: number;
}
export interface InferenceRound {
  phase?: string;
  usage: {
    inputTokens?: number | null; outputTokens?: number | null; cachedTokens?: number | null;
    cacheWriteTokens?: number | null; processedPromptTokens?: number | null;
    promptMs?: number | null; generationMs?: number | null;
    generationTokensPerSecond?: number | null; promptTokensPerSecond?: number | null;
  };
  firstVisibleAnswerMs?: number | null; reasoningMs?: number | null;
  firstTokenMs: number | null; durationMs: number; complete: boolean;
}
export interface ResponseMetrics { preparationMs?: number | null; toolsMs?: number | null; approvalMs?: number | null; model: string; rounds: InferenceRound[]; totalMs: number }
export interface InstalledModel {
  id: string; filename: string; path: string; bytes: number; repo: string | null;
  revision: string | null; files: string[]; complete: boolean; received?: number;
  projectorFilename?: string | null; projectorPath?: string | null;
}
export interface HubFile { filename: string; bytes: number; sha256: string }
export interface HubRepo { repo: string; revision: string; files: HubFile[] }
export interface HubSearchHit { id: string; downloads: number | null; likes: number | null }
export interface ModelMetadata {
  architecture: string | null; contextLength: number | null; blockCount: number | null;
  embeddingLength: number | null; headCount: number | null; headCountKv: number | null;
  keyLength: number | null; valueLength: number | null; fileBytes: number;
}
export interface Preferences {
  runtimePath: string; modelPath: string; projectorPath?: string; temperature: number; topP: number; maxTokens: number;
  reasoningMode?: 'auto' | 'quick' | 'think';
  generativeUi?: boolean;
  systemPrompt: string;
}
export interface RuntimeStatus { phase: 'stopped' | 'loading' | 'ready' | 'error'; message: string; modelPath: string | null; loadedConfig?: RuntimeConfig | null; gpuOffload?: { layers: number; totalLayers: number } | null; projector?: { path: string; offload: boolean; fileBytes: number } | null }
export type ToolSupport = 'unknown' | 'supported' | 'unsupported';
export interface ModelControls {
  source: 'configured' | 'provider-reported';
  endpointFingerprint?: string | null;
  temperature: ToolSupport; topP: ToolSupport; reasoningEffort: ToolSupport;
  toolChoice: ToolSupport; parallelToolCalls: ToolSupport; structuredOutput: ToolSupport;
  tokenLimitField: 'protocol-default' | 'max-tokens' | 'max-completion-tokens' | 'max-output-tokens';
}
export interface RemoteModel { supportsImages?: boolean; id: string; contextLength: number | null; maxOutputTokens: number | null; toolSupport: ToolSupport; controls?: ModelControls | null }
export type ApiFormat = 'openai-chat-completions' | 'openai-responses' | 'chatgpt-subscription' | 'grok-subscription' | 'freetoken-openai' | 'claude-messages';
export interface ProviderConnection {
  id: string; name: string; apiFormat: ApiFormat; baseUrl: string;
  verified: boolean; lastTestedAt: number | null; models: RemoteModel[]; hasApiKey: boolean;
}
export interface ProviderDraft {
  id?: string | null; name: string; apiFormat: ApiFormat; baseUrl: string;
  apiKey?: string | null; models: RemoteModel[];
}
export interface ProviderTestResult { verified: boolean; modelListSupported: boolean; models: string[]; message: string }
export interface ModelSelection { providerId: string | null; modelId: string }
export interface Bootstrap {
  conversations: Conversation[]; config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus;
  rememberedTools: RememberedTools; providers: ProviderConnection[]; preferredModel: ModelSelection;
}
export interface RememberedTools { sources: string[]; tools: ToolSelection[]; accessMode?: AccessMode }
export interface McpInputField {
  name: string; required?: boolean;
  kind: 'text' | 'number' | 'integer' | 'boolean' | 'select' | 'multi';
  title?: string; description?: string; default?: unknown;
  format?: string; minLength?: number; maxLength?: number;
  minimum?: number; maximum?: number;
  options?: { value: string; label?: string }[];
  minItems?: number; maxItems?: number;
}
export interface McpInputForm { message: string; fields: McpInputField[] }
export interface ToolApproval { purpose?: string | null; id: string; kind?: 'tool' | 'ask_user' | 'mcp_input' | 'plan_approval'; connector: string; localServerName?: string | null; name: string; arguments: Record<string, unknown>; form?: McpInputForm | null; closed?: boolean }
export interface AskUserApproval extends ToolApproval { kind: 'ask_user'; }
export interface ContextBreakdown {
  messages: number; systemTools: number; mcpTools: number; systemPrompt: number; skills: number; metaContext: number;
}
export interface ContextUsage {
  inputTokens: number; responseReserve: number; contextLength: number; estimated?: boolean;
  cachedTokens?: number | null; cacheWriteTokens?: number | null; outputTokens?: number | null;
  cacheHitRate?: number | null; promptMs?: number | null; generationMs?: number | null;
  breakdown?: ContextBreakdown; avgCacheHitRate?: number | null;
}
export interface PreflightBreakdown { presentation?: {profile:string;minimumInputTokens:number;separateRequest:boolean;estimated:boolean;contextLength:number;responseReserve:number}; offeredToolCount?: number; total: number; instructions: number; tools: number; history: number; scratchpad?: number; draft: number; responseReserve: number; contextLength: number; exact: boolean; fits: boolean; notice?: string; overflow?: string }
export interface Preset {
  id: string;
  name: string;
  description: string;
  sources: string[];
  mcp: boolean;
  systemTime: boolean;
  skills: boolean;
  harness: string[];
}
export interface Checkpoint {
  conversationId: string;
  cutoff: number;
  artifactId: string;
  createdAt: number;
}
export interface CompactionOutcome {
  checkpoint: Checkpoint;
  droppedMessages: number;
  keptMessages: number;
  note: string;
}
export interface CompactionStatus {
  auto: boolean;
  keepLast: number;
  checkpoint: Checkpoint | null;
}
export interface Project { id: string; name: string; path: string }
export interface TaskMeta { projectId: string | null; archived: boolean; pinned: boolean }
export interface WorkspaceIndex { projects: Project[]; tasks: Record<string, TaskMeta> }
export type RunState =
  | 'preparing'
  | 'generating'
  | 'awaiting_approval'
  | 'executing_tools'
  | 'preparing_next_round'
  | 'paused'
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
  outcome?: RunOutcome | null;
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

export type ToolStreamEvent =
  | { type: 'started'; toolCallId: string; toolName: string; command?: string; language?: string; cwd?: string }
  | { type: 'outputChunk'; toolCallId: string; stream: 'stdout' | 'stderr' | string; chunk: string }
  | { type: 'finished'; toolCallId: string; exitCode?: number | null; durationMs: number; error?: string | null };
export interface Hit {
  conversationId: string;
  conversationTitle: string;
  messageId: string;
  role: string;
  excerpt: string;
  createdAt: number;
}

export interface TodoItem {
  text: string;
  status: 'pending' | 'in_progress' | 'completed' | string;
  updatedAt: number;
}

export type RunOutcome = { reason: 'completed' | 'user_cancelled' | 'user_paused' | 'app_restart' | 'provider_failure' | 'budget_exhausted' | 'permission_denied' | 'outcome_unknown' | 'blocked'; resumable: boolean; checkpoint: string | null };
export interface ChatEvent {
  outcome?: RunOutcome;
  discardStep?: string;
  presentation?: PresentationRecord;
  notice?: string;
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
  elapsedSecs?: number;
  roundIndex?: number;
  toolStream?: ToolStreamEvent;
}
export interface ToolView { name: string; description: string; inputSchema: Record<string, unknown> }
export interface ToolSelection { connectorId: string; toolName: string }
export interface ConnectorView {
  id: string; description: string; url: string; authType: 'none' | 'apiKey' | 'oauth' | 'local';
  connected: boolean; hasCredential: boolean; tools: ToolView[]; connectionError?: string;
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
export interface ExecutionPolicy {
  enabled: boolean; timeoutSeconds: number; workingDirectory: string;
  environment: Record<string, string>; alwaysAsk: boolean;
  memoryLimitMib?: number | null;
}
export interface WeatherConfig { enabled: boolean; location: string; temperatureUnit: 'celsius' | 'fahrenheit' }
export interface GitReview {
  branch: string; branches: string[]; status: string; diff: string; repository?: boolean;
  files?: { path: string; status: string; previousPath?: string }[]; truncated?: boolean; stagedTree?: string | null;
}
export type GitAction = { action: 'stage' | 'unstage'; paths: string[] } | { action: 'commit'; message: string; expectedTree: string; confirmed: boolean } | { action: 'createBranch'; name: string } | { action: 'history' };
export interface GitHistoryEntry { id: string; shortId: string; author: string; date: string; subject: string }
export interface ExecutionConfig {
  pythonPath: string; nodePath: string; powershellPath: string;
  bashPath?: string; bash?: ExecutionPolicy; python?: ExecutionPolicy; pythonVenv?: string;
}
export interface WebSearchConfig { provider: 'searxng' | 'google'; searxngBaseUrl: string; hasGoogleApiKey?: boolean; googleApiKey?: string; clearGoogleApiKey?: boolean; googleCxId: string; searchFallbackEnabled: boolean; enabled?: boolean; mode?: 'fast' | 'normal' | 'deep' }
export interface HardwareStatus {
  logicalCpus: number; memoryTotalBytes: number | null; memoryAvailableBytes: number | null;
  gpuStatus: string; sampledAt: number;
  gpus: { name: string; uuid: string; memoryUsedMib: number | null; memoryTotalMib: number | null; utilizationPercent: number | null; driverVersion: string }[];
}
export type AccessMode = 'ask' | 'autoApprove' | 'fullAccess';
export interface ConversationTools { sources: string[]; tools: ToolSelection[]; accessMode: AccessMode }
export type CapabilityKind = 'Model' | 'Tool' | 'Skill' | 'Session' | 'Sandbox' | 'Storage' | 'Loop' | 'Ui';
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
export interface Hit {
  conversationId: string;
  conversationTitle: string;
  messageId: string;
  role: string;
  excerpt: string;
  createdAt: number;
}

export interface AutomationConfig {
  browserEnabled: boolean; computerEnabled: boolean; browserPackagePath: string;
  browserExecutablePath: string; browserChannel: 'chrome' | 'msedge'; headless: boolean;
}
export interface LspServerConfig {
  id: string;
  languageId: string;
  command: string;
  enabled: boolean;
  arguments: string[];
  environment: Record<string, string>;
  initializationOptions: unknown;
  settings: unknown;
  timeoutSeconds: number;
  allowedCommands?: string[];
}
export interface LspConfig {
  enabled: boolean;
  alwaysAsk: boolean;
  servers: LspServerConfig[];
}

export interface Hook {
  id: string;
  event: 'pre_tool_use' | 'post_tool_use' | 'run_finished';
  matcher: string;
  command: string;
  timeoutSecs: number;
  enabled: boolean;
}

export interface DebuggerPolicy { requestTimeoutSeconds: number; idleTimeoutSeconds: number; lifetimeSeconds: number }
export interface ProfilerConfig { nativeExecutable: string; exportExecutable: string; exportProfile: string }
export interface HttpCredentialView { id: string; name: string; header: string; origin: string }
export interface DatabaseProfile {
  id: string; name: string; engine: 'postgres' | 'mysql'; host: string; port: number;
  database: string; username: string; tls: 'verify-full' | 'require' | 'disable';
  scratchProfileId: string | null; dumpExecutable: string; restoreExecutable: string;
}

export interface PermissionRuleView {
  action: 'tool' | 'command' | 'path' | 'host';
  resource: string;
  effect: 'allow' | 'deny' | 'ask';
}

export interface SnapshotFileView {
  path: string;
  sha256: string;
  size: number;
  deleted: boolean;
}

export interface SnapshotView {
  id: string;
  conversationId: string;
  step: number;
  createdAt: number;
  files: SnapshotFileView[];
  skipped: string[];
}

export interface StagedRevertView {
  id: string;
  conversationId: string;
  snapshotId: string;
  paths: string[];
  createdAt: number;
}

export interface RevertReportView {
  restored: string[];
  deleted: string[];
  overwritten: string[];
  failed: { path: string; reason: string }[];
}

export interface PluginView {
  name: string;
  description: string;
  command: string[];
  arguments: Record<string, unknown>;
  required: string[];
  timeoutSeconds: number;
  path: string;
  status: 'ok' | 'error';
  error?: string | null;
}
