import { BuiltinActionType, type ActionEvent } from '@openuidev/react-lang';
import { buttonAction } from './catalog';

export type PresentationAction =
  | { type: 'open_url'; url: string }
  | { type: 'continue_conversation'; message: string };

/** No tool/provider/Tauri command names cross this allowlist. */
export function presentationAction(event: ActionEvent): PresentationAction | null {
  if (event.type === BuiltinActionType.OpenUrl) {
    const result = buttonAction.safeParse({ type: 'open_url', url: event.params?.url });
    return result.success && result.data?.type === 'open_url' ? result.data : null;
  }
  if (event.type !== BuiltinActionType.ContinueConversation) return null;
  const message = event.params?.message ?? event.humanFriendlyMessage;
  if (typeof message !== 'string' || !message.trim() || message.length > 4_000) return null;
  return { type: 'continue_conversation', message };
}
