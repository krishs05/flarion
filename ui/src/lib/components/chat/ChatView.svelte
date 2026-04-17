<script lang="ts">
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import {
    chatStore,
    getActive,
    appendMessage,
    appendToLastMessage,
    finalizeLastMessage,
    newChat,
    persistChats
  } from '$lib/stores/chats.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { streamChatCompletion } from '$lib/api/streaming';
  import type { ChatCompletionRequest } from '$lib/api/types';
  import ModelParams from './ModelParams.svelte';
  import MessageList from './MessageList.svelte';
  import ChatInput from './ChatInput.svelte';
  import ChatHistoryPanel from './ChatHistoryPanel.svelte';

  let active = $derived(getActive());
  let streaming = $state(false);
  let error = $state<string | null>(null);
  let streamingIndex = $state<number | null>(null);

  let model = $state(connection.modelId ?? '');
  let temperature = $state(settings.defaultParams.temperature);
  let topP = $state(settings.defaultParams.topP);
  let maxTokens = $state(settings.defaultParams.maxTokens);

  $effect(() => {
    if (!model && connection.modelId) {
      model = connection.modelId;
    }
  });

  function handleParamsChange(p: {
    model: string;
    temperature: number;
    topP: number;
    maxTokens: number;
  }) {
    model = p.model;
    temperature = p.temperature;
    topP = p.topP;
    maxTokens = p.maxTokens;
  }

  let abortController: AbortController | null = null;

  function handleStop() {
    abortController?.abort();
  }

  async function handleSend(text: string) {
    error = null;
    let chat = active;
    if (!chat) {
      chat = newChat(model || connection.modelId || 'unknown');
    }

    const chatId = chat.id;
    const effectiveModel = model || chat.model;
    chat.model = effectiveModel;

    appendMessage(chatId, { role: 'user', content: text });
    const afterUser = chatStore.chats.find((c) => c.id === chatId);
    if (!afterUser) return;

    const request: ChatCompletionRequest = {
      model: effectiveModel,
      messages: afterUser.messages.map(({ role, content }) => ({ role, content })),
      temperature,
      top_p: topP,
      max_tokens: maxTokens,
      stream: true
    };

    appendMessage(chatId, { role: 'assistant', content: '' });
    const afterAssistant = chatStore.chats.find((c) => c.id === chatId);
    streamingIndex = afterAssistant ? afterAssistant.messages.length - 1 : null;
    streaming = true;

    const start = performance.now();
    let firstTokenAt: number | null = null;
    let tokenCount = 0;

    abortController = new AbortController();

    try {
      for await (const chunk of streamChatCompletion(settings.baseUrl, request, abortController.signal)) {
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
      if (!(e instanceof DOMException && e.name === 'AbortError')) {
        error = e instanceof Error ? e.message : String(e);
      }
      persistChats();
    } finally {
      streaming = false;
      streamingIndex = null;
      abortController = null;
    }
  }

  let inputDisabled = $derived(streaming || !connection.connected);
</script>

<div class="h-full flex min-w-0">
  <ChatHistoryPanel />

  <div class="flex-1 flex flex-col min-w-0">
    <ModelParams
      {model}
      {temperature}
      {topP}
      {maxTokens}
      onChange={handleParamsChange}
    />

    {#if error}
      <div class="px-5 py-2.5 bg-signal/10 border-b border-signal/30 flex items-center gap-2">
        <AlertTriangle class="w-4 h-4 text-signal shrink-0" />
        <span class="font-mono text-xs text-signal">error: {error}</span>
      </div>
    {/if}

    {#if active}
      <MessageList messages={active.messages} {streamingIndex} />
    {:else}
      <div class="flex-1 flex items-center justify-center">
        <div class="text-center">
          <div class="font-mono text-[11px] uppercase tracking-[0.16em] text-graphite">
            {connection.connected ? 'no chat selected' : 'flarion offline'}
          </div>
          <div class="mt-2 text-sm text-graphite-hi">
            {connection.connected
              ? 'create a new chat or send a message below'
              : 'check the endpoint in settings'}
          </div>
        </div>
      </div>
    {/if}

    <ChatInput disabled={inputDisabled} streaming={streaming} onSend={handleSend} onStop={handleStop} />
  </div>
</div>
