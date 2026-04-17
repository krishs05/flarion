import type { ChatCompletionChunk, ChatCompletionRequest } from './types';
import { FlarionApiError } from './types';

async function parseError(response: Response): Promise<FlarionApiError> {
  const text = await response.text();
  try {
    const body = JSON.parse(text);
    return new FlarionApiError(response.status, body, body.error?.message ?? text);
  } catch {
    return new FlarionApiError(response.status, text, text || response.statusText);
  }
}

/**
 * Stream a chat completion. Yields ChatCompletionChunk objects as they arrive.
 * Terminates when the server sends `data: [DONE]` or the stream ends.
 */
export async function* streamChatCompletion(
  baseUrl: string,
  request: ChatCompletionRequest,
  signal?: AbortSignal
): AsyncGenerator<ChatCompletionChunk, void, unknown> {
  const res = await fetch(`${baseUrl}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ...request, stream: true }),
    signal
  });

  if (!res.ok) throw await parseError(res);
  if (!res.body) throw new Error('Response has no body');

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });

      let boundary = buffer.indexOf('\n\n');
      while (boundary !== -1) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        boundary = buffer.indexOf('\n\n');

        const dataLines = frame
          .split('\n')
          .filter((l) => l.startsWith('data:'))
          .map((l) => l.slice(5).trimStart());

        if (dataLines.length === 0) continue;
        const data = dataLines.join('\n');

        if (data === '[DONE]') return;

        try {
          yield JSON.parse(data) as ChatCompletionChunk;
        } catch (e) {
          console.warn('Failed to parse SSE chunk:', data, e);
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
