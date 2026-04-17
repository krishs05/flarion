import type {
  ChatCompletionRequest,
  ChatCompletionResponse,
  HealthResponse,
  ModelsResponse,
  ApiErrorBody
} from './types';
import { FlarionApiError } from './types';

async function parseError(response: Response): Promise<FlarionApiError> {
  const text = await response.text();
  try {
    const body = JSON.parse(text) as ApiErrorBody;
    return new FlarionApiError(response.status, body, body.error?.message ?? text);
  } catch {
    return new FlarionApiError(response.status, text, text || response.statusText);
  }
}

export async function getHealth(baseUrl: string, signal?: AbortSignal): Promise<HealthResponse> {
  const res = await fetch(`${baseUrl}/health`, { signal });
  if (!res.ok) throw await parseError(res);
  return res.json();
}

export async function listModels(baseUrl: string, signal?: AbortSignal): Promise<ModelsResponse> {
  const res = await fetch(`${baseUrl}/v1/models`, { signal });
  if (!res.ok) throw await parseError(res);
  return res.json();
}

export async function chatCompletion(
  baseUrl: string,
  request: ChatCompletionRequest,
  signal?: AbortSignal
): Promise<ChatCompletionResponse> {
  const res = await fetch(`${baseUrl}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ...request, stream: false }),
    signal
  });
  if (!res.ok) throw await parseError(res);
  return res.json();
}

/**
 * Fetch raw Prometheus metrics text. Many servers dedicate this to a
 * trusted listener (Phase 2d); if unreachable we just return null.
 */
export async function getMetrics(
  baseUrl: string,
  signal?: AbortSignal
): Promise<string | null> {
  try {
    const res = await fetch(`${baseUrl}/metrics`, { signal });
    if (!res.ok) return null;
    return await res.text();
  } catch {
    return null;
  }
}
