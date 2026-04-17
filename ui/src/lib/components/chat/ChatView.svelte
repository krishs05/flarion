<script lang="ts">
  import { getActive, appendMessage, appendToLastMessage, finalizeLastMessage, newChat, persistChats } from '$lib/stores/chats.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { streamChatCompletion } from '$lib/api/streaming';
  import type { ChatCompletionRequest } from '$lib/api/types';
  import ModelParams from './ModelParams.svelte';
  import MessageList from './MessageList.svelte';
  import ChatInput from './ChatInput.svelte';

  let active = $derived(getActive());
  let streaming = $state(false);
  let error = $state<string | null>(null);
  let streamingIndex = $state<number | null>(null);

  let temperature = $state(settings.defaultParams.temperature);
  let topP = $state(settings.defaultParams.topP);
  let maxTokens = $state(settings.defaultParams.maxTokens);

  function handleParamsChange(p: { temperature: number; topP: number; maxTokens: number }) {
    temperature = p.temperature;
    topP = p.topP;
    maxTokens = p.maxTokens;
  }

  async function handleSend(text: string) {
    error = null;
    let chat = active;
    if (!chat) {
      const model = connection.modelId ?? 'unknown';
      chat = newChat(model);
    }

    const chatId = chat.id;
    appendMessage(chatId, { role: 'user', content: text });
    appendMessage(chatId, { role: 'assistant', content: '' });
    streamingIndex = chat.messages.length - 1;
    streaming = true;

    const request: ChatCompletionRequest = {
      model: chat.model,
      messages: chat.messages
        .slice(0, -1)
        .map(({ role, content }) => ({ role, content })),
      temperature,
      top_p: topP,
      max_tokens: maxTokens,
      stream: true
    };

    const start = performance.now();
    let firstTokenAt: number | null = null;
    let tokenCount = 0;

    try {
      for await (const chunk of streamChatCompletion(settings.baseUrl, request)) {
        const delta = chunk.choices[0]?.delta?.content;
        if (delta) {
          if (firstTokenAt === null) firstTokenAt = performance.now();
          appendToLastMessage(chatId, delta);
          tokenCount += delta.length / 4;
        }
      }

      const end = performance.now();
      const durationMs = end - start;
      const ttft = firstTokenAt !== null ? firstTokenAt - start : durationMs;
      const tokensPerSecond = tokenCount > 0 ? (tokenCount / durationMs) * 1000 : 0;

      finalizeLastMessage(chatId, {
        ttft: Math.round(ttft),
        tokensPerSecond,
        totalTokens: Math.round(tokenCount),
        durationMs: Math.round(durationMs)
      });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      persistChats();
    } finally {
      streaming = false;
      streamingIndex = null;
    }
  }

  let inputDisabled = $derived(streaming || !connection.connected);
</script>

<div class="h-full flex flex-col">
  <ModelParams
    {temperature}
    {topP}
    {maxTokens}
    onChange={handleParamsChange}
  />

  {#if error}
    <div class="px-4 py-2 bg-signal/20 border-b border-signal text-signal font-mono text-xs">
      error: {error}
    </div>
  {/if}

  {#if active}
    <MessageList messages={active.messages} {streamingIndex} />
  {:else}
    <div class="flex-1 flex items-center justify-center text-graphite font-mono text-sm uppercase tracking-wider">
      {connection.connected ? 'click "+ new chat" or send a message to begin' : 'connect to flarion to start chatting'}
    </div>
  {/if}

  <ChatInput disabled={inputDisabled} onSend={handleSend} />
</div>
