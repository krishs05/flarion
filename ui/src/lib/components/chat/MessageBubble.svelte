<script lang="ts">
  import type { Message } from '$lib/stores/chats.svelte';
  import { renderMarkdown } from '$lib/utils/markdown';
  import MetricsBadge from './MetricsBadge.svelte';
  import Copy from '@lucide/svelte/icons/copy';
  import Check from '@lucide/svelte/icons/check';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import User from '@lucide/svelte/icons/user';
  import Info from '@lucide/svelte/icons/info';

  interface Props {
    message: Message;
    streaming?: boolean;
  }

  let { message, streaming = false }: Props = $props();
  let html = $derived(renderMarkdown(message.content || ''));
  let copied = $state(false);

  async function copyContent() {
    try {
      await navigator.clipboard.writeText(message.content);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {
      /* ignore */
    }
  }
</script>

{#if message.role === 'user'}
  <div class="group flex justify-end animate-[fade-in_240ms_var(--ease-out-soft)]">
    <div class="max-w-[78%] flex flex-col items-end gap-1.5">
      <div
        class="rounded-2xl rounded-tr-md px-4 py-2.5 text-[14.5px] leading-relaxed
          bg-gradient-to-br from-ember/[0.18] to-ember/[0.08]
          border border-ember/20 text-frost-hi"
      >
        <div class="prose prose-invert max-w-none prose-sm">
          {@html html}
        </div>
      </div>
      <div class="flex items-center gap-2 pr-1 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          type="button"
          onclick={copyContent}
          class="text-graphite hover:text-frost transition-colors"
          aria-label="Copy"
        >
          {#if copied}
            <Check class="w-3.5 h-3.5 text-lime" />
          {:else}
            <Copy class="w-3.5 h-3.5" />
          {/if}
        </button>
        <User class="w-3 h-3 text-graphite" />
      </div>
    </div>
  </div>
{:else if message.role === 'assistant'}
  <div class="group flex gap-3 animate-[fade-in_240ms_var(--ease-out-soft)]">
    <div
      class="w-8 h-8 rounded-lg shrink-0 flex items-center justify-center
        bg-surface-hi border border-wire {streaming ? 'ring-ember' : ''}"
    >
      <Sparkles class="w-4 h-4 text-ember" />
    </div>
    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2 mb-1.5">
        <span class="font-mono text-[10px] uppercase tracking-[0.14em] text-graphite">
          assistant
        </span>
        {#if streaming}
          <span class="font-mono text-[10px] uppercase tracking-[0.14em] text-ember">
            · streaming
          </span>
          <span class="inline-flex gap-0.5">
            <span class="w-1 h-1 rounded-full bg-ember animate-[pulse-soft_1.2s_ease-in-out_infinite]"></span>
            <span class="w-1 h-1 rounded-full bg-ember animate-[pulse-soft_1.2s_ease-in-out_infinite_100ms]"></span>
            <span class="w-1 h-1 rounded-full bg-ember animate-[pulse-soft_1.2s_ease-in-out_infinite_200ms]"></span>
          </span>
        {/if}
      </div>
      <div class="prose prose-invert max-w-none prose-sm font-sans text-[14.5px] leading-relaxed">
        {@html html}
        {#if streaming && !message.content}
          <span class="text-graphite italic">thinking…</span>
        {/if}
      </div>
      {#if message.metrics || message.content}
        <div class="mt-2 flex items-center gap-3">
          {#if message.metrics}
            <MetricsBadge metrics={message.metrics} />
          {/if}
          {#if message.content}
            <button
              type="button"
              onclick={copyContent}
              class="opacity-0 group-hover:opacity-100 transition-opacity
                font-mono text-[10px] uppercase tracking-wider text-graphite hover:text-frost
                inline-flex items-center gap-1.5"
            >
              {#if copied}
                <Check class="w-3 h-3 text-lime" />
                copied
              {:else}
                <Copy class="w-3 h-3" />
                copy
              {/if}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{:else}
  <div class="flex gap-3 animate-[fade-in_240ms_var(--ease-out-soft)]">
    <div class="w-8 h-8 rounded-lg shrink-0 bg-surface border border-wire flex items-center justify-center">
      <Info class="w-4 h-4 text-graphite" />
    </div>
    <div class="flex-1 min-w-0 font-mono text-xs text-graphite">
      <div class="uppercase tracking-wider mb-1 text-graphite-hi">system</div>
      <div>{message.content}</div>
    </div>
  </div>
{/if}
