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
  inferenceSlots: number;
}

export const defaultRuntimeConfig: RuntimeConfig = {
  contextLength: 131072, gpuLayers: -1, cpuThreads: 6, batchSize: 512,
  microBatchSize: 128, flashAttention: true, cacheTypeK: 'q8_0', cacheTypeV: 'q8_0',
  offloadKvCache: true, mmap: true, inferenceSlots: 1,
};

export interface Conversation {
  id: string; title: string; updatedAt: number;
  providerId: string | null; modelId: string | null; providerSelectionRequired: boolean;
}
export interface Message {
  error?: string | null;
  metrics?: TurnMetrics | null;
  id: string; conversationId: string; role: 'user' | 'assistant' | 'tool';
  content: string; reasoning: string; status: 'complete' | 'streaming' | 'interrupted' | 'error'; createdAt: number;
}
export interface InstalledModel {
  id: string; filename: string; path: string; bytes: number; repo: string | null;
  revision: string | null; files: string[]; complete: boolean; received?: number;
  projectorFilename?: string | null; projectorPath?: string | null;
  external?: boolean;
  /** Pinned expectations for every managed shard and projector. */
  expected?: HubFile[];
  /** Computed during scanning: expected files are present with matching sizes. */
  verified?: boolean;
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
  systemPrompt: string;
}
export interface RuntimeStatus { phase: 'stopped' | 'loading' | 'ready' | 'error'; message: string; modelPath: string | null; loadedConfig?: RuntimeConfig | null; launchConfig?: RuntimeConfig | null; loadedContext?: number | null; gpuOffload?: { layers: number; totalLayers: number } | null }
export type ToolSupport = 'unknown' | 'supported' | 'unsupported';
export interface RemoteModel { supportsImages?: boolean; id: string; contextLength: number | null; maxOutputTokens: number | null; toolSupport: ToolSupport }
export interface ProviderConnection {
  id: string; name: string; apiFormat: 'openai-chat-completions' | 'chatgpt-subscription' | 'grok-subscription' | 'freetoken-openai'; baseUrl: string;
  verified: boolean; lastTestedAt: number | null; models: RemoteModel[]; hasApiKey: boolean;
}
export interface ModelSelection { providerId: string | null; modelId: string }
export interface Bootstrap {
  conversations: Conversation[]; config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus;
  rememberedTools: RememberedTools; providers: ProviderConnection[]; preferredModel: ModelSelection;
}
export interface RememberedTools { sources: string[]; tools: ToolSelection[]; accessMode?: AccessMode; interactionMode?: InteractionMode }
/** Ask/Plan/Act — what the model may do this turn; separate from accessMode, which gates approvals. */
export type InteractionMode = 'ask' | 'plan' | 'act';
/** One backend-computed row of the effective-tools listing. `available` means
 *  the next turn would offer it; `permittedInMode` means the interaction mode
 *  lets it run; `requiresApproval` means the permission mode would still ask. */
export interface EffectiveTool {
  name: string; source: string;
  available: boolean; enabled: boolean; permittedInMode: boolean; requiresApproval: boolean;
  blockedReason?: string;
}
export interface ToolApproval { id: string; kind?: 'tool' | 'ask_user'; connector: string; localServerName?: string | null; name: string; arguments: Record<string, unknown> }
export interface AskUserApproval extends ToolApproval { kind: 'ask_user'; }
export interface ContextUsage { inputTokens: number; outputTokens?: number | null; cachedInputTokens?: number | null; responseReserve: number; contextLength: number; estimated?: boolean }
export interface PreflightBreakdown { total: number; instructions: number; tools: number; history: number; scratchpad?: number; draft: number; responseReserve: number; contextLength: number; exact: boolean; fits: boolean; notice?: string; overflow?: string }
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
// Isolated task workspaces — a git worktree or bounded snapshot copy of the
// project that one conversation works in until its changes are reviewed,
// applied back, or discarded. Review/recovery only, not a security sandbox.
export interface TaskExclusion { path: string; reason: string }
export interface TaskWorkspace {
  id: string; conversationId: string; sourceRoot: string;
  kind: 'git-worktree' | 'snapshot';
  isolatedRoot: string; branch: string | null; baselineCommit: string;
  status: 'active' | 'applied' | 'discarded';
  exclusions: TaskExclusion[]; createdAt: number; updatedAt: number;
}
export interface TaskCheckpoint { commit: string | null; label: string; createdAt: number; uncommitted: boolean }
export interface TaskFileChange {
  path: string; oldPath: string | null;
  status: 'added' | 'modified' | 'deleted' | 'renamed';
  binary: boolean; size: number;
  baselineHash: string | null; taskHash: string | null; destinationHash: string | null;
  destinationChanged: boolean; conflict: boolean;
}
export interface AppliedFile {
  path: string; action: 'created' | 'written' | 'deleted';
  wroteSha256: string | null; backup: string | null; taskVersion: string | null;
}
export interface TaskApply { id: string; taskId: string; createdAt: number; backupDir: string; files: AppliedFile[]; undoneAt: number | null }
export interface TaskReview {
  taskId: string; sourceRoot: string; isolatedRoot: string;
  kind: 'git-worktree' | 'snapshot'; branch: string | null; baselineCommit: string;
  exclusions: TaskExclusion[]; files: TaskFileChange[]; checkpoints: TaskCheckpoint[]; applies: TaskApply[];
}
export interface TaskFileDiff { text: string | null; truncated: boolean; binary: boolean; reason: string | null }
export interface TaskApplyResult { applied: string[]; keptDestination: string[]; skippedConflicts: string[]; backupDir: string; applyId: string }
export interface TaskUndoResult { restored: string[]; conflicts: string[] }
export type TaskConflictResolution = 'take-task' | 'keep-destination';
export interface Project { id: string; name: string; path: string }
export interface TaskMeta { projectId: string | null; archived: boolean; pinned: boolean }
export interface WorkspaceIndex { projects: Project[]; tasks: Record<string, TaskMeta> }
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

export interface TurnMetrics { ttftSecs?: number | null; promptTokPerSec?: number | null; genTokPerSec?: number | null }
export interface ChatEvent {
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
  metrics?: TurnMetrics;
  /** Terminal event of a Plan-mode turn — offer "Implement plan". */
  planReady?: boolean;
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
export interface ExecutionConfig { pythonPath: string; nodePath: string; powershellPath: string }
export type IsolationMode = 'unsandboxed' | 'enforced';
export interface IsolationSettings {
  mode: IsolationMode;
  /** Sandboxed processes get network access. Off means no network at all, including loopback. */
  network: boolean;
  /** LAN reach — requires network and an explicit opt-in. */
  lan: boolean;
  /** Opt into LPAC — stricter, but some interpreters cannot read their registry keys. */
  lpac: boolean;
  /** Extra directories (interpreters, toolchains) the container can read and execute. */
  toolchainDirs: string[];
  processLimit: number;
  memoryMb: number;
}
export interface IsolationStatus {
  available: boolean;
  kind: string | null;
  reason: string;
  policy: IsolationSettings;
  grants: string[];
}
export type BrowserMode = 'off' | 'auto' | 'preferred';
export interface BrowserConfig { mode: BrowserMode; maxActions: number }
export interface WebSearchConfig { provider: 'searxng' | 'google'; searxngBaseUrl: string; hasGoogleApiKey: boolean; googleCxId: string; searchFallbackEnabled: boolean }
export interface HardwareStatus {
  logicalCpus: number; memoryTotalBytes: number | null; memoryAvailableBytes: number | null;
  gpuStatus: string; sampledAt: number;
  gpus: { name: string; uuid: string; memoryUsedMib: number | null; memoryTotalMib: number | null; utilizationPercent: number | null; driverVersion: string }[];
}
export type AccessMode = 'ask' | 'autoApprove' | 'fullAccess';
export interface ConversationTools { sources: string[]; tools: ToolSelection[]; accessMode: AccessMode; interactionMode?: InteractionMode }
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

export type CriterionOrigin = 'user' | 'agent';
export type VerificationKind = 'command' | 'browser' | 'research' | 'calculation' | 'data' | 'manual';
export type VerificationEnvironment = 'native-local' | 'task-worktree' | 'snapshot' | 'mocked';
export type VerificationResult = 'passed' | 'failed' | 'unavailable' | 'inconclusive';
export type CompletionState = 'work-completed' | 'checks-passed' | 'checks-failed' | 'partially-verified' | 'needs-review';

export interface TaskCriterion {
  id: string; conversationId: string; text: string; origin: CriterionOrigin; createdAt: number; updatedAt: number;
}
export interface VerificationRecord {
  id: string; conversationId: string; criterionId?: string | null;
  kind: VerificationKind; method: string; command?: string | null;
  environment: VerificationEnvironment | string;
  startedAt: number; endedAt: number; exitCode?: number | null;
  /** Tail excerpt only — never the full log. Use verificationRecordOutput. */
  outputExcerpt: string; outputArtifact?: string | null; outputTruncated: boolean;
  artifacts: unknown; result: VerificationResult; limitations: string;
  fingerprint?: string | null; createdAt: number;
  /** Computed at read time: the workspace changed since this record ran. */
  stale: boolean;
}
export interface VerificationSummary {
  criteria: TaskCriterion[];
  records: VerificationRecord[];
  state: { state: CompletionState; summary: string };
}
export interface VerificationOutput { text: string; truncated: boolean; note?: string }

export type MemoryScope = string; // 'user' | `project:${string}` | `conversation:${string}`
export interface MemoryFact {
  id: string; scope: MemoryScope; fact: string; origin: 'user' | 'agent' | 'survey' | string;
  reason?: string | null; sourceConversationId?: string | null;
  sourcePath?: string | null; sourceHash?: string | null; sourceBranch?: string | null;
  pinned: boolean; excluded: boolean; stale: boolean; createdAt: number; updatedAt: number;
}
export interface DocCollection {
  id: string; name: string; root: string; fileCount: number; chunkCount: number;
  indexedAt: number | null; error: string | null; createdAt: number;
}
export interface DocHit {
  collectionId: string; collection: string; path: string;
  lineStart: number; lineEnd: number; snippet: string;
}
export interface Recipe {
  id: string;
  name: string;
  prompt: string;
  intervalMinutes: number | null;
  enabled: boolean;
  nextRunAt: number | null;
  lastRunAt: number | null;
  createdAt: number;
}
export interface Task {
  id: string;
  conversationId: string;
  content: string;
  interactionMode: string;
  status: string;
  position: number | null;
  runId: string | null;
  waitReason: string | null;
  error: string | null;
  nextAction: string | null;
  parentTaskId: string | null;
  createdAt: number;
  startedAt: number | null;
  endedAt: number | null;
}

export interface TaskTransition {
  taskId: string;
  fromStatus: string;
  toStatus: string;
  reason: string | null;
  at: number;
}

export interface ContextPreviewItem {
  kind: string; label: string; tokens: number; reason: string; source?: string | null; id?: string | null;
}
export interface ContextPreview { items: ContextPreviewItem[]; totalTokens: number; budgetTokens: number }
export interface UsedMemory { id: string; fact: string }

export interface CaseResult {
  name: string; status: 'passed' | 'failed' | 'unsupported' | 'untested';
  latencyMs: number; tokensPerSecond?: number | null; error?: string | null; detail?: string | null;
}
export interface Qualification {
  id: string; identityHash: string; stale: boolean; modelId: string; modelPath?: string | null;
  providerId?: string | null; runtimeVersion?: string | null; config: unknown;
  startedAt: number; endedAt?: number | null; cases: CaseResult[];
  status: 'passed' | 'partial' | 'failed' | 'cancelled';
}
export type QualificationEvent =
  | { kind: 'case'; name: string; status: string; result?: CaseResult | null }
  | { kind: 'complete'; record: Qualification };
export interface Readiness {
  step: string; message: string;
  nextAction?: { label: string; target: string } | null;
}
export interface Profile {
  id: string; label: string; preset: string; interactionMode: InteractionMode;
  recommended: boolean; note?: string | null;
}
