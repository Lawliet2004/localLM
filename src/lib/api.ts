import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Bootstrap, ChatEvent, ConnectorView, Conversation, Message, Preferences, RuntimeConfig, RuntimeStatus } from './types';

export const nativeAvailable = isTauri();
export const api = {
  listConnectors: () => invoke<ConnectorView[]>('list_connectors'),
  connectConnector: (id: string, apiToken?: string) => invoke<ConnectorView>('connect_connector', { id, apiToken }),
  disconnectConnector: (id: string, forget = false) => invoke<void>('disconnect_connector', { id, forget }),
  signInConnector: (id: string) => invoke<ConnectorView>('sign_in_connector', { id }),
  cancelConnectorSignIn: () => invoke<void>('cancel_connector_sign_in'),
  bootstrap: () => invoke<Bootstrap>('bootstrap'),
  createConversation: () => invoke<Conversation>('create_conversation'),
  renameConversation: (id: string, title: string) => invoke<void>('rename_conversation', { id, title }),
  deleteConversation: (id: string) => invoke<void>('delete_conversation', { id }),
  messages: (id: string) => invoke<Message[]>('get_messages', { id }),
  saveConfig: (config: RuntimeConfig) => invoke<void>('save_runtime_config', { config }),
  savePreferences: (preferences: Preferences) => invoke<void>('save_preferences', { preferences }),
  loadModel: () => invoke<RuntimeStatus>('load_model'),
  unloadModel: () => invoke<RuntimeStatus>('unload_model'),
  runtimeStatus: () => invoke<RuntimeStatus>('runtime_status'),
  sendMessage: (conversationId: string, content: string, onEvent: (event: ChatEvent) => void, connectorIds: string[] = []) => {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<void>('send_message', { conversationId, content, channel, connectorIds });
  },
  cancelGeneration: () => invoke<void>('cancel_generation'),
  resolveToolApproval: (id: string, allow: boolean) => invoke<void>('resolve_tool_approval', { id, allow }),
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
