<script lang="ts">
  import type { Message } from '$lib/stores/chats.svelte';
  import MessageBubble from './MessageBubble.svelte';

  interface Props {
    messages: Message[];
    streamingIndex: number | null;
  }

  let { messages, streamingIndex }: Props = $props();

  let container: HTMLDivElement;

  $effect(() => {
    if (container && messages.length > 0) {
      void messages[messages.length - 1]?.content;
      queueMicrotask(() => {
        container.scrollTop = container.scrollHeight;
      });
    }
  });
</script>

<div bind:this={container} class="flex-1 overflow-y-auto">
  <div class="max-w-4xl mx-auto px-6 py-8 space-y-6">
    {#if messages.length === 0}
      <div class="h-full py-16 flex flex-col items-center justify-center text-center">
        <div class="w-14 h-14 rounded-2xl bg-ember/10 border border-ember/30 flex items-center justify-center mb-4 ring-ember">
          <span class="font-mono text-2xl text-ember">▸</span>
        </div>
        <h3 class="font-sans text-xl font-semibold text-frost-hi">Start a conversation</h3>
        <p class="mt-2 font-mono text-xs text-graphite uppercase tracking-wider max-w-md">
          send a message to begin — responses stream token-by-token
        </p>
      </div>
    {:else}
      {#each messages as msg, i (i)}
        <MessageBubble message={msg} streaming={streamingIndex === i} />
      {/each}
    {/if}
  </div>
</div>
