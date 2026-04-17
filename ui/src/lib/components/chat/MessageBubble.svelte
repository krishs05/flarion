<script lang="ts">
  import type { Message } from '$lib/stores/chats.svelte';
  import { renderMarkdown } from '$lib/utils/markdown';
  import MetricsBadge from './MetricsBadge.svelte';

  interface Props {
    message: Message;
    streaming?: boolean;
  }

  let { message, streaming = false }: Props = $props();

  let html = $derived(renderMarkdown(message.content || ''));
</script>

{#if message.role === 'user'}
  <div class="flex justify-end">
    <div class="max-w-[80%] bg-carbon rounded-md p-3 font-sans text-frost">
      <div class="prose prose-invert max-w-none prose-sm">
        {@html html}
      </div>
    </div>
  </div>
{:else if message.role === 'assistant'}
  <div class="border-l-2 border-wire pl-4 py-1">
    <div class="font-mono text-xs text-graphite uppercase tracking-wider mb-2">
      assistant
      {#if streaming}
        <span class="text-ember animate-pulse">· streaming</span>
      {/if}
    </div>
    <div class="prose prose-invert max-w-none prose-sm font-sans">
      {@html html}
      {#if streaming && !message.content}
        <span class="text-graphite italic">thinking...</span>
      {/if}
    </div>
    {#if message.metrics}
      <MetricsBadge metrics={message.metrics} />
    {/if}
  </div>
{:else}
  <div class="border-l-2 border-graphite pl-4 py-1 text-xs font-mono text-graphite">
    <div class="uppercase tracking-wider mb-1">system</div>
    <div>{message.content}</div>
  </div>
{/if}
