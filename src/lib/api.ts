import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type {
  ArtifactRecord,
  Bootstrap,
  Capability,
  ChatEvent,
  ConnectorView,
  Conversation,
  ConversationTools,
  ExecutionConfig,
  Fact,
  HardwareStatus,
  Message,
  ModelSelection,
  Plugin,
  Preferences,
  Preset,
  ProviderConnection,
  ProviderDraft,
  ProviderFormat,
  ProviderTestResult,
  RememberedTools,
  RunEvent,
  RunRecord,
  RuntimeConfig,
  RuntimeStatus,
  ScanVerdict,
  Schedule,
  SkillDependencyStatus,
  SkillUpdateStatus,
  SkillView,
  SubagentRun,
  Todo,
  ToolSelection,
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
  readLocalConnector: (id: string) => invoke<LocalServerConfig>('read_local_connector', { id }),
  saveLocalConnector: (server: LocalServerConfig) => invoke<void>('save_local_connector', { server }),
  removeLocalConnector: (id: string) => invoke<void>('remove_local_connector', { id }),
  listInstalledRuntimes: () => invoke<{ id: string; path: string; complete: boolean; problem: string | null }[]>('list_installed_runtimes'),
  runtimeDownloadInfo: () => invoke<{ bytes: number; requiredBytes: number; availableBytes: number; destination: string }>('runtime_download_info'),
  runtimeInstallStatus: () => invoke<ModelInstallStatus>('runtime_install_status'),
  installRuntime: () => invoke<ModelInstallStatus>('install_runtime'),
  cancelRuntimeInstall: () => invoke<void>('cancel_runtime_install'),
  modelDownloadInfo: () => invoke<ModelDownloadInfo>('model_download_info'),
  modelInstallStatus: () => invoke<ModelInstallStatus>('model_install_status'),
  installModel: () => invoke<ModelInstallStatus>('install_model'),
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
  listPresets: () => invoke<Preset[]>('list_presets'),
  getPreset: (conversationId: string) => invoke<Preset>('get_preset', { conversationId }),
  setPreset: (conversationId: string, preset: string) => invoke<Preset>('set_preset', { conversationId, preset }),
  getConversationRuns: (conversationId: string) => invoke<RunRecord[]>('get_conversation_runs', { conversationId }),
  forkSession: (conversationId: string, throughMessageId: string) => invoke<Conversation>('fork_session', { conversationId, throughMessageId }),
  replaySession: (conversationId: string) => invoke<Record<string, unknown>>('replay_session', { conversationId }),
  searchSessions: (query: string, limit?: number) => invoke<{ conversationId: string; conversationTitle: string; messageId: string; role: string; excerpt: string; createdAt: number }[]>('search_sessions', { query, limit }),
  getTodos: (conversationId: string) => invoke<Todo[]>('get_todos', { conversationId }),
  getGoal: (conversationId: string) => invoke<string | null>('get_goal', { conversationId }),
  listSubagentRuns: (conversationId: string) => invoke<SubagentRun[]>('list_subagent_runs', { conversationId }),
  interruptSubagent: (childRunId: string) => invoke<boolean>('interrupt_subagent', { childRunId }),
  listFacts: (scope: string, limit?: number) => invoke<Fact[]>('list_facts', { scope, limit }),
  teachFact: (scope: string, fact: string) => invoke<Fact>('teach_fact_cmd', { scope, fact }),
  forgetFact: (id: string) => invoke<boolean>('forget_fact', { id }),
  ingestRepo: (scope: string) => invoke<Fact>('ingest_repo', { scope }),
  listSchedules: () => invoke<Schedule[]>('list_schedules'),
  saveSchedule: (schedule: Schedule) => invoke<Schedule>('save_schedule', { schedule }),
  deleteSchedule: (id: string) => invoke<boolean>('delete_schedule', { id }),
  runScheduleNow: (id: string) => invoke<string>('run_schedule_now', { id }),
  webhookState: () => invoke<{ enabled: boolean; port: number; hasToken: boolean }>('webhook_state'),
  setWebhook: (enabled: boolean, port?: number) => invoke<{ enabled: boolean; port: number; hasToken: boolean }>('set_webhook', { enabled, port }),
  rotateWebhookToken: () => invoke<string>('rotate_webhook_token'),
  sandboxStatus: () => invoke<{ provider: string; warning: string; docker: string }>('sandbox_status'),
  setSandboxProvider: (providerName: string, image?: string) => invoke<void>('set_sandbox_provider', { providerName, image }),
  listPlugins: () => invoke<Plugin[]>('list_plugins'),
  installPlugin: (path: string) => invoke<Plugin>('install_plugin', { path }),
  setPluginEnabled: (name: string, enabled: boolean) => invoke<Plugin[]>('set_plugin_enabled', { name, enabled }),
  removePlugin: (name: string) => invoke<boolean>('remove_plugin', { name }),
  scanPlugin: (path: string) => invoke<ScanVerdict>('scan_plugin', { path }),
  testPlugin: (path: string) => invoke<Record<string, unknown>>('test_plugin', { path }),
  compactConversation: (conversationId: string, keepLast?: number) => invoke<{ conversationId: string; cutoff: number; artifactId: string; createdAt: number }>('compact_conversation_cmd', { conversationId, keepLast }),
  compactionStatus: (conversationId: string) => invoke<{ checkpoint: unknown; auto: boolean; keepLast: number }>('compaction_status', { conversationId }),
  setCompactionAuto: (auto: boolean, keepLast?: number) => invoke<void>('set_compaction_auto', { auto, keepLast }),
  providerFormats: () => invoke<ProviderFormat[]>('provider_formats'),
  getArtifact: (id: string) => invoke<ArtifactRecord | null>('get_artifact', { id }),
  listConversationArtifacts: (conversationId: string) => invoke<ArtifactRecord[]>('list_conversation_artifacts', { conversationId }),
  saveConfig: (config: RuntimeConfig) => invoke<void>('save_runtime_config', { config }),
  savePreferences: (preferences: Preferences) => invoke<void>('save_preferences', { preferences }),
  loadModel: () => invoke<RuntimeStatus>('load_model'),
  unloadModel: () => invoke<RuntimeStatus>('unload_model'),
  runtimeStatus: () => invoke<RuntimeStatus>('runtime_status'),
  sendMessage: (conversationId: string, content: string, onEvent: (event: ChatEvent) => void, connectorIds: string[] = [], connectorTools: ToolSelection[] = []) => {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<void>('send_message', { conversationId, content, channel, connectorIds, connectorTools });
  },
  cancelGeneration: () => invoke<void>('cancel_generation'),
  resolveToolApproval: (id: string, allow: boolean) => invoke<void>('resolve_tool_approval', { id, allow }),
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
