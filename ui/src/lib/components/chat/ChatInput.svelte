<script lang="ts">
  interface Props {
    disabled?: boolean;
    onSend: (text: string) => void;
  }

  let { disabled = false, onSend }: Props = $props();

  let text = $state('');
  let textarea: HTMLTextAreaElement;

  function resize() {
    if (!textarea) return;
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 200) + 'px';
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
</script>

<div class="border-t border-wire bg-carbon p-4">
  <div class="flex gap-2 items-end">
    <textarea
      bind:this={textarea}
      bind:value={text}
      onkeydown={handleKeydown}
      {disabled}
      placeholder={disabled ? 'connect to a flarion server to chat' : 'message (enter to send, shift+enter for newline)'}
      rows="1"
      class="flex-1 bg-midnight border border-wire rounded-md px-3 py-2 font-sans text-frost
        placeholder:text-graphite focus:border-ember outline-none resize-none
        disabled:opacity-40 disabled:cursor-not-allowed"
    ></textarea>
    <button
      onclick={handleSend}
      disabled={disabled || !text.trim()}
      class="px-4 py-2 bg-ember text-midnight font-mono text-sm rounded-md
        hover:shadow-[0_0_12px_rgba(255,107,43,0.3)] transition-shadow
        disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:shadow-none"
    >
      send
    </button>
  </div>
</div>
