import type { ApiFormat, ProviderConnection, RemoteModel } from './types';

export const MAX_PROVIDER_MODELS = 2048;

/** Shortcuts only; every connection keeps an editable URL and protocol.
 * Endpoint sources: openrouter.ai/docs/api/api-reference/models/get-models,
 * console.groq.com/docs/openai, docs.together.ai/docs/openai-api-compatibility. */
export const PROVIDER_PRESETS: { id: string; name: string; baseUrl: string; apiFormat: ApiFormat }[] = [
  { id: 'openrouter', name: 'OpenRouter', baseUrl: 'https://openrouter.ai/api/v1', apiFormat: 'openai-chat-completions' },
  { id: 'groq', name: 'Groq', baseUrl: 'https://api.groq.com/openai/v1', apiFormat: 'openai-chat-completions' },
  { id: 'together', name: 'Together', baseUrl: 'https://api.together.xyz/v1', apiFormat: 'openai-chat-completions' },
];

/** Local inference engines speaking the OpenAI chat-completions protocol. */
export const LOCAL_PROVIDER_PRESETS: { id: string; name: string; baseUrl: string; apiFormat: ApiFormat }[] = [
  { id: 'ollama', name: 'Ollama', baseUrl: 'http://localhost:11434/v1', apiFormat: 'openai-chat-completions' },
  { id: 'lm-studio', name: 'LM Studio', baseUrl: 'http://localhost:1234/v1', apiFormat: 'openai-chat-completions' },
];

const LOOPBACK_HOSTS = new Set(['localhost', '127.0.0.1', '::1']);

function isLoopbackHost(host: string): boolean {
  return LOOPBACK_HOSTS.has(host.replace(/^\[|\]$/g, '').toLowerCase());
}

export interface BaseUrlCheck { normalized: string; loopback: boolean }

/** Validate a provider base URL: HTTPS for remote hosts, HTTP only for real
 * loopback (localhost, 127.0.0.1, ::1). Credentials, queries, and fragments
 * are rejected — mirroring the native `validate_base_url` rules. */
export function parseProviderBaseUrl(raw: string): BaseUrlCheck | { error: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { error: 'Enter the provider base URL.' };
  let url: URL;
  try { url = new URL(trimmed); }
  catch { return { error: 'The base URL is not valid. Include the scheme and host, for example https://api.example.com/v1.' }; }
  if (!url.hostname) return { error: 'The base URL must include a host.' };
  if (url.username || url.password || trimmed.includes('?') || trimmed.includes('#')) {
    return { error: 'The base URL must not contain credentials, a query, or a fragment.' };
  }
  const loopback = isLoopbackHost(url.hostname);
  if (url.protocol !== 'https:' && !(loopback && url.protocol === 'http:')) {
    return { error: 'Remote providers require HTTPS. HTTP is allowed only for localhost, 127.0.0.1, or ::1.' };
  }
  return { normalized: trimmed.replace(/\/+$/, ''), loopback };
}

export function isLoopbackBaseUrl(raw: string): boolean {
  const parsed = parseProviderBaseUrl(raw);
  return 'loopback' in parsed ? parsed.loopback : false;
}

export function isSubscriptionFormat(format: string): boolean {
  return format === 'chatgpt-subscription' || format === 'grok-subscription';
}

/** Mirrors the native `requires_api_key`: only Claude endpoints always
 * authenticate with x-api-key — loopback included. OpenAI-compatible and
 * custom providers may run keyless even on remote hosts; subscriptions use
 * their imported session instead. */
export function providerRequiresApiKey(format: string, _baseUrl: string): boolean {
  void _baseUrl;
  if (isSubscriptionFormat(format)) return false;
  return format === 'claude-messages';
}

/** Whether a remote model can actually serve chat: provider verified, the
 * model has an explicit context capacity, and a credential exists when the
 * endpoint requires one. Subscription providers are excluded — their hosted
 * inference stays gated by the local-only policy. */
export function remoteModelReady(provider: ProviderConnection, model: RemoteModel | null | undefined): boolean {
  return Boolean(model)
    && !('error' in parseProviderBaseUrl(provider.baseUrl))
    && provider.verified
    && Boolean(model!.contextLength)
    && !isSubscriptionFormat(provider.apiFormat)
    && (provider.hasApiKey || !providerRequiresApiKey(provider.apiFormat, provider.baseUrl));
}

/** Documented Chat Completions families; unknown/pro/private subscription protocols stay Auto. */
export function supportsReasoningEffort(model: string): boolean {
  return ['gpt-5', 'gpt-5-mini', 'gpt-5-nano', 'gpt-5.1', 'gpt-5.2', 'gpt-5.4', 'gpt-5.5', 'o1', 'o3', 'o3-mini', 'o4-mini'].some(name => model === name || model.startsWith(`${name}-20`));
}
