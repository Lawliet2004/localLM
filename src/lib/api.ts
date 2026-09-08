import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Bootstrap, ChatEvent, ConnectorView, Conversation, ConversationTools, ExecutionConfig, HardwareStatus, Message, Preferences, RuntimeConfig, RuntimeStatus, SkillView, ToolSelection } from './types';

export const nativeAvailable = isTauri();
export interface ModelInstallStatus { busy: boolean; phase: string; received: number; total: number; path: string | null; error: string | null }
export interface ModelDownloadInfo { filename: string; bytes: number; sha256: string; destination: string; availableBytes: number; requiredBytes: number; destinationExists: boolean }
export const api = {
  saveLocalConnector: (server: { id: string; name: string; executable: string; arguments: string[]; workingDirectory: string; environment: Record<string, string> }) => invoke<void>('save_local_connector', { server }),
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
  listConnectors: () => invoke<ConnectorView[]>('list_connectors'),
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
  exportConversation: (id: string, path: string) => invoke<void>('export_conversation', { id, path }),
  messages: (id: string) => invoke<Message[]>('get_messages', { id }),
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
