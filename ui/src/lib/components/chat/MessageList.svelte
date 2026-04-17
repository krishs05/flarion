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

<div bind:this={container} class="flex-1 overflow-y-auto p-6 space-y-6">
  {#if messages.length === 0}
    <div class="h-full flex items-center justify-center text-graphite font-mono text-sm uppercase tracking-wider">
      send a message to begin
    </div>
  {:else}
    {#each messages as msg, i (i)}
      <MessageBubble message={msg} streaming={streamingIndex === i} />
    {/each}
  {/if}
</div>
