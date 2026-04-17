<script lang="ts">
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import StopCircle from '@lucide/svelte/icons/stop-circle';
  import Kbd from '$lib/components/ui/Kbd.svelte';

  interface Props {
    disabled?: boolean;
    streaming?: boolean;
    onSend: (text: string) => void;
    onStop?: () => void;
  }

  let { disabled = false, streaming = false, onSend, onStop }: Props = $props();

  let text = $state('');
  let textarea: HTMLTextAreaElement;

  function resize() {
    if (!textarea) return;
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 220) + 'px';
  }

  function handleSend() {
    const trimmed = text.trim();
    if (!trimmed || disabled) return;
    onSend(trimmed);
    text = '';
    queueMicrotask(resize);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  $effect(() => {
    void text;
    resize();
  });

  let canSend = $derived(!disabled && text.trim().length > 0);
</script>

<div class="shrink-0 p-4 bg-gradient-to-t from-midnight to-transparent">
  <div class="max-w-4xl mx-auto">
    <div
      class="flex gap-2 items-end p-2 rounded-2xl border border-wire bg-surface/80
        backdrop-blur-xl transition-colors focus-within:border-wire-hi focus-within:bg-surface"
    >
      <textarea
        bind:this={textarea}
        bind:value={text}
        onkeydown={handleKeydown}
        {disabled}
        placeholder={disabled
          ? 'connect to a flarion server to chat'
          : 'message flarion…'}
        rows="1"
        class="flex-1 min-h-[36px] max-h-[220px] bg-transparent px-3 py-2 font-sans text-[14.5px]
          text-frost placeholder:text-graphite outline-none resize-none leading-relaxed
          disabled:opacity-40 disabled:cursor-not-allowed"
      ></textarea>

      {#if streaming && onStop}
        <button
          type="button"
          onclick={onStop}
          class="w-9 h-9 rounded-xl flex items-center justify-center shrink-0
            bg-signal/15 border border-signal/40 text-signal hover:bg-signal/25 transition-colors"
          aria-label="Stop streaming"
        >
          <StopCircle class="w-4 h-4" />
        </button>
      {:else}
        <button
          type="button"
          onclick={handleSend}
          disabled={!canSend}
          class="w-9 h-9 rounded-xl flex items-center justify-center shrink-0
            bg-ember text-midnight transition-all
            hover:bg-ember-soft hover:shadow-[0_0_20px_-4px_rgba(255,107,43,0.7)]
            disabled:bg-wire disabled:text-graphite disabled:shadow-none
            disabled:cursor-not-allowed"
          aria-label="Send"
        >
          <ArrowUp class="w-4 h-4" />
        </button>
      {/if}
    </div>

    <div class="mt-2 flex items-center justify-between font-mono text-[10px] uppercase tracking-wider text-graphite">
      <div class="flex items-center gap-2">
        <Kbd>↵</Kbd>
        <span>send</span>
        <span class="text-wire">·</span>
        <Kbd>⇧</Kbd>
        <Kbd>↵</Kbd>
        <span>newline</span>
      </div>
      <div class="hidden sm:block">
        streaming via flarion openai-compatible api
      </div>
    </div>
  </div>
</div>
