<script lang="ts">
  interface Props {
    data: unknown;
    status?: number;
    error?: string | null;
  }

  let { data, status, error }: Props = $props();

  let pretty = $derived(
    data === null || data === undefined
      ? ''
      : typeof data === 'string'
        ? data
        : JSON.stringify(data, null, 2)
  );

  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(pretty);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }
</script>

{#if error}
  <div class="bg-signal/20 border border-signal rounded-md p-3 font-mono text-sm text-signal">
    {error}
  </div>
{:else if data !== null && data !== undefined}
  <div class="relative">
    <div class="flex items-center justify-between mb-2">
      <span class="font-mono text-xs text-graphite">
        {#if status}<span class="text-cyan-flare">{status}</span> ·{/if}
        response
      </span>
      <button
        onclick={copy}
        class="font-mono text-xs text-graphite hover:text-ember transition-colors"
      >
        {copied ? 'copied!' : 'copy'}
      </button>
    </div>
    <pre class="bg-carbon border border-wire rounded-md p-4 font-mono text-sm text-frost overflow-x-auto"><code>{pretty}</code></pre>
  </div>
{/if}
