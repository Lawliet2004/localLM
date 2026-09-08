import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Bootstrap, ChatEvent, Conversation, Message, Preferences, RuntimeConfig, RuntimeStatus } from './types';

export const nativeAvailable = isTauri();
export const api = {
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
  sendMessage: (conversationId: string, content: string, onEvent: (event: ChatEvent) => void) => {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<void>('send_message', { conversationId, content, channel });
  },
  cancelGeneration: () => invoke<void>('cancel_generation'),
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
