import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type {
  Project, TaskMeta, WorkspaceIndex,
  ArtifactRecord,
  Bootstrap,
  Capability,
  ChatEvent,
  TodoItem,
  ConnectorView,
  Conversation,
  ConversationTools,
  ExecutionConfig,
  HardwareStatus,
  Message,
  ModelSelection,
  Preferences,
  ProviderConnection,
  ProviderDraft,
  ProviderTestResult,
  RememberedTools,
  RunEvent,
  RunRecord,
  RuntimeConfig,
  RuntimeStatus,
  SessionEvent,
  ForkSessionResult,
  Hit,
  SkillDependencyStatus,
  SkillUpdateStatus,
  SkillView,
  SubscriptionStatus,
  CliDetectionResult,
  ToolSelection,
  PreflightBreakdown,
  Preset,
  CompactionOutcome,
  CompactionStatus,
  WebSearchConfig,
} from './types';

export const nativeAvailable = isTauri();
export interface LocalServerConfig {
  id: string;
  name: string;
  executable: string;
  arguments: string[];
  workingDirectory: string;
  environment: Record<string, string>;
}
export interface ModelInstallStatus {
  busy: boolean;
  phase: string;
  received: number;
  total: number;
  path: string | null;
  error: string | null;
}
export interface ModelDownloadInfo {
  filename: string;
  bytes: number;
  sha256: string;
  destination: string;
  availableBytes: number;
  requiredBytes: number;
  destinationExists: boolean;
}

export const api = {
  workspaceIndex: () => invoke<WorkspaceIndex>('workspace_index'),
  saveProject: (project: Project) => invoke<WorkspaceIndex>('save_project', { project }),
  removeProject: (id: string) => invoke<WorkspaceIndex>('remove_project', { id }),
  saveTaskMeta: (id: string, meta: TaskMeta) => invoke<WorkspaceIndex>('save_task_meta', { id, meta }),
  workspaceInspect: (path: string, directory: boolean) => invoke<{ entries?: {name: string; kind: string}[]; content?: string; truncated?: boolean }>('workspace_inspect', { path, directory }),
  workspaceGit: (branch?: string) => invoke<{ branch: string; branches: string[]; status: string; diff: string }>('workspace_git', { branch }),
  workspaceCommand: (command: string, onChunk: (event: {stream: string; chunk: string}) => void) => {
    const channel = new Channel<{stream: string; chunk: string}>(); channel.onmessage = onChunk;
    return invoke<{stdout: string; stderr: string; exitCode: number | null; error?: string; durationMs: number}>('workspace_command', { command, channel });
  },
  readLocalConnector: (id: string) => invoke<LocalServerConfig>('read_local_connector', { id }),
  saveLocalConnector: (server: LocalServerConfig) => invoke<void>('save_local_connector', { server }),
  removeLocalConnector: (id: string) => invoke<void>('remove_local_connector', { id }),
  listInstalledRuntimes: () => invoke<{ id: string; path: string; complete: boolean; problem: string | null }[]>('list_installed_runtimes'),
  runtimeDownloadInfo: () => invoke<{ bytes: number; requiredBytes: number; availableBytes: number; destination: string }>('runtime_download_info'),
  runtimeInstallStatus: () => invoke<ModelInstallStatus>('runtime_install_status'),
  installRuntime: () => invoke<ModelInstallStatus>('install_runtime'),
  cancelRuntimeInstall: () => invoke<void>('cancel_runtime_install'),
  modelDownloadInfo: (filename?: string) => invoke<ModelDownloadInfo>('model_download_info', { filename }),
  listInstalledModels: () => invoke<import('./types').InstalledModel[]>('list_installed_models'),
  searchHuggingFace: (query: string, token?: string) => invoke<import('./types').HubSearchHit[]>('search_hugging_face', { query, token }),
  huggingFaceFiles: (repo: string, token?: string) => invoke<import('./types').HubRepo>('hugging_face_files', { repo, token }),
  downloadHuggingFaceModel: (repo: string, revision: string, filename: string, token?: string, projector?: string | null) => invoke<ModelInstallStatus>('download_hugging_face_model', { repo, revision, filename, token, projector }),
  deleteInstalledModel: (id: string) => invoke<void>('delete_installed_model', { id }),
  useInstalledModel: (id: string) => invoke<RuntimeStatus>('use_installed_model', { id }),
  readModelMetadata: (path: string) => invoke<import('./types').ModelMetadata>('read_model_metadata', { path }),
  modelInstallStatus: () => invoke<ModelInstallStatus>('model_install_status'),
  installModel: (filename?: string) => invoke<ModelInstallStatus>('install_model', { filename }),
  cancelModelInstall: () => invoke<void>('cancel_model_install'),
  readRuntimeLog: () => invoke<{ content: string; truncated: boolean }>('read_runtime_log'),
  hasDaytonaKey: () => invoke<boolean>('has_daytona_key'),
  saveDaytonaKey: (key: string) => invoke<void>('save_daytona_key', { key }),
  forgetDaytonaKey: () => invoke<void>('forget_daytona_key'),
  retryDaytonaCleanup: (name: string) => invoke<void>('retry_daytona_cleanup', { name }),
  pendingDaytonaOperations: () => invoke<{ name: string; sandboxId: string | null; createdAt: number; cleanupError: string | null }[]>('pending_daytona_operations'),
  hardwareStatus: () => invoke<HardwareStatus>('hardware_status'),
  getExecutionConfig: () => invoke<ExecutionConfig>('get_execution_config'),
  saveExecutionConfig: (config: ExecutionConfig) => invoke<void>('save_execution_config', { config }),
  detectInterpreters: () => invoke<ExecutionConfig>('detect_interpreters'),
  testInterpreter: (path: string) => invoke<string>('test_interpreter', { path }),
  getWorkspace: () => invoke<{ path: string }>('get_workspace'),
  setWorkspace: (path: string) => invoke<void>('set_workspace', { path }),
  listSkills: () => invoke<SkillView[]>('list_skills'),
  installSkill: (id: string) => invoke<void>('install_skill', { id }),
  removeSkill: (id: string) => invoke<void>('remove_skill', { id }),
  setSkillActive: (id: string, active: boolean) => invoke<void>('set_skill_active', { id, active }),
  readSkillFile: (id: string, path: string) => invoke<string>('read_skill_file', { id, path }),
  skillDependencies: (id: string) => invoke<SkillDependencyStatus[]>('skills_skill_dependencies', { id }),
  skillUpdateStatus: (id: string) => invoke<SkillUpdateStatus>('skill_update_status', { id }),
  listConnectors: () => invoke<ConnectorView[]>('list_connectors'),
  listLocalConnectors: () => invoke<ConnectorView[]>('list_local_connectors'),
  connectConnector: (id: string, apiToken?: string) => invoke<ConnectorView>('connect_connector', { id, apiToken }),
  disconnectConnector: (id: string, forget = false) => invoke<void>('disconnect_connector', { id, forget }),
  signInConnector: (id: string) => invoke<ConnectorView>('sign_in_connector', { id }),
  cancelConnectorSignIn: () => invoke<void>('cancel_connector_sign_in'),
  bootstrap: () => invoke<Bootstrap>('bootstrap'),
  createConversation: () => invoke<Conversation>('create_conversation'),
  renameConversation: (id: string, title: string) => invoke<void>('rename_conversation', { id, title }),
  deleteConversation: (id: string) => invoke<void>('delete_conversation', { id }),
  conversationTools: (id: string) => invoke<ConversationTools>('get_conversation_tools', { id }),
  saveConversationTools: (id: string, tools: ConversationTools) => invoke<void>('save_conversation_tools', { id, tools }),
  rememberedTools: () => invoke<RememberedTools>('get_remembered_tools'),
  saveRememberedTools: (tools: RememberedTools) => invoke<void>('save_remembered_tools', { tools }),
  listProviders: () => invoke<ProviderConnection[]>('list_providers'),
  saveProvider: (provider: ProviderDraft) => invoke<ProviderConnection>('save_provider', { draft: provider }),
  deleteProvider: (id: string) => invoke<void>('delete_provider', { id }),
  testProvider: (id: string) => invoke<ProviderTestResult>('test_provider', { id }),
  testProviderInference: (id: string, modelId: string) => invoke<ProviderTestResult>('test_provider_inference', { id, modelId }),
  listProviderModels: (id: string) => invoke<ProviderConnection>('list_provider_models', { id }),
  detectSubscriptionCli: (provider: string) => invoke<CliDetectionResult>('detect_subscription_cli', { provider }),
  importSubscriptionCli: (provider: string) => invoke<SubscriptionStatus>('import_subscription_cli', { provider }),
  getSubscriptionStatus: (provider: string) => invoke<SubscriptionStatus>('get_subscription_status', { provider }),
  saveManualSubscriptionToken: (provider: string, token: string, refreshToken?: string | null, accountId?: string | null) =>
    invoke<SubscriptionStatus>('save_manual_subscription_token', { provider, token, refreshToken, accountId }),
  disconnectSubscription: (provider: string) => invoke<void>('disconnect_subscription', { provider }),
  startSubscriptionSignIn: (provider: string) => invoke<SubscriptionStatus>('start_subscription_sign_in', { provider }),
  cancelSubscriptionSignIn: () => invoke<void>('cancel_subscription_sign_in'),
  preferredModel: () => invoke<ModelSelection>('preferred_model'),
  savePreferredModel: (selection: ModelSelection) => invoke<void>('save_preferred_model', { selection }),
  saveConversationModel: (id: string, selection: ModelSelection) => invoke<void>('save_conversation_model', { id, selection }),
  exportConversation: (id: string, path: string) => invoke<void>('export_conversation', { id, path }),
  messages: (id: string) => invoke<Message[]>('get_messages', { id }),
  getRun: (id: string) => invoke<RunRecord | null>('get_run', { id }),
  getConversationRun: (conversationId: string) => invoke<RunRecord | null>('get_conversation_run', { conversationId }),
  getRunEvents: (runId: string) => invoke<RunEvent[]>('get_run_events', { runId }),
  listCapabilities: (conversationId?: string | null) => invoke<Capability[]>('list_capabilities', { conversationId }),
  setCapabilityEnabled: (id: string, enabled: boolean) => invoke<Capability[]>('set_capability_enabled', { id, enabled }),
  dumpConfig: (conversationId?: string | null) => invoke<Record<string, unknown>>('dump_config', { conversationId }),
  contextPreflight: (conversationId: string | null, draft: string, preset: string | null, connectorIds: string[], connectorTools: ToolSelection[], planMode = false) =>
    invoke<{ available: boolean; reason?: string; breakdown?: PreflightBreakdown }>('context_preflight', { conversationId, draft, preset, connectorIds, connectorTools, planMode }),
  listPresets: () => invoke<Preset[]>('list_presets'),
  getPreset: (conversationId: string) => invoke<Preset>('get_preset', { conversationId }),
  setPreset: (conversationId: string, preset: string) => invoke<Preset>('set_preset', { conversationId, preset }),
  compactConversation: (conversationId: string) => invoke<CompactionOutcome>('compact_conversation_cmd', { conversationId }),
  compactionStatus: (conversationId: string) => invoke<CompactionStatus>('compaction_status', { conversationId }),
  setCompactionAuto: (conversationId: string, enabled: boolean) => invoke<void>('set_compaction_auto', { conversationId, enabled }),
  getArtifact: (id: string) => invoke<ArtifactRecord | null>('get_artifact', { id }),
  listConversationArtifacts: (conversationId: string) => invoke<ArtifactRecord[]>('list_conversation_artifacts', { conversationId }),
  saveConfig: (config: RuntimeConfig) => invoke<void>('save_runtime_config', { config }),
  savePreferences: (preferences: Preferences) => invoke<void>('save_preferences', { preferences }),
  loadModel: () => invoke<RuntimeStatus>('load_model'),
  unloadModel: () => invoke<RuntimeStatus>('unload_model'),
  runtimeStatus: () => invoke<RuntimeStatus>('runtime_status'),
  sendMessage: (conversationId: string, content: string, onEvent: (event: ChatEvent) => void, connectorIds: string[] = [], connectorTools: ToolSelection[] = [], planMode = false) => {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<void>('send_message', { conversationId, content, channel, connectorIds, connectorTools, planMode });
  },
  cancelGeneration: () => invoke<void>('cancel_generation'),
  resolveToolApproval: (id: string, allow: boolean) => invoke<void>('resolve_tool_approval', { id, allow }),
  resolveAskUser: (id: string, choice: string | null) => invoke<void>('resolve_ask_user', { id, choice }),
  getTodos: (conversationId: string) => invoke<TodoItem[]>('get_todos', { conversationId }),
  getGoal: (conversationId: string) => invoke<string | null>('get_goal', { conversationId }),
  getSessionEvents: (conversationId: string, fromSeq?: number | null, limit?: number | null) =>
    invoke<SessionEvent[]>('get_session_events', { conversationId, fromSeq, limit }),
  forkSession: (conversationId: string, fromSeq: number) =>
    invoke<ForkSessionResult>('fork_session', { conversationId, fromSeq }),
  replaySession: (conversationId: string) =>
    invoke<SessionEvent[]>('replay_session', { conversationId }),
  searchSessions: (query: string, limit?: number | null) =>
    invoke<Hit[]>('search_sessions', { query, limit }),
  getWebSearchConfig: () => invoke<WebSearchConfig>('get_web_search_config'),
  saveWebSearchConfig: (config: WebSearchConfig) => invoke<void>('save_web_search_config', {
    provider: config.provider,
    searxngBaseUrl: config.searxngBaseUrl,
    googleApiKey: config.googleApiKey,
    googleCxId: config.googleCxId,
    searchFallbackEnabled: config.searchFallbackEnabled,
  }),
  webSearchHealth: () => invoke<Record<string, unknown>>('web_search_health'),
};

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === 'object') {
    const obj = error as Record<string, unknown>;
    if (typeof obj.message === 'string') return obj.message;
    if (typeof obj.error === 'string') return obj.error;
  }
  const text = String(error);
  return text === '[object Object]' ? 'An unexpected error occurred.' : text;
}

