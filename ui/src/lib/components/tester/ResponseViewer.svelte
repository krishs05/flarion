<script lang="ts">
  import Copy from '@lucide/svelte/icons/copy';
  import Check from '@lucide/svelte/icons/check';
  import AlertCircle from '@lucide/svelte/icons/alert-circle';
  import Badge from '$lib/components/ui/Badge.svelte';

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

  let statusTone = $derived.by(() => {
    if (!status) return 'neutral';
    if (status < 300) return 'lime';
    if (status < 400) return 'cyan';
    if (status < 500) return 'amber';
    return 'signal';
  });
</script>

{#if error}
  <div class="card p-4 border-signal/40 bg-signal/[0.05]">
    <div class="flex items-start gap-3">
      <AlertCircle class="w-5 h-5 text-signal shrink-0 mt-0.5" />
      <div class="min-w-0">
        {#if status}
          <Badge tone="signal" size="xs">{status}</Badge>
        {/if}
        <div class="mt-1 font-mono text-sm text-signal break-words">{error}</div>
      </div>
    </div>
  </div>
{:else if data !== null && data !== undefined}
  <div class="card overflow-hidden">
    <div class="px-4 py-2.5 border-b border-wire flex items-center justify-between bg-carbon/60">
      <div class="flex items-center gap-2">
        {#if status}
          <Badge tone={statusTone as 'lime' | 'cyan' | 'amber' | 'signal'}>{status}</Badge>
        {/if}
        <span class="font-mono text-[10px] uppercase tracking-wider text-graphite">response</span>
      </div>
      <button
        type="button"
        onclick={copy}
        class="font-mono text-[10px] uppercase tracking-wider text-graphite hover:text-frost
          transition-colors inline-flex items-center gap-1.5"
      >
        {#if copied}
          <Check class="w-3 h-3 text-lime" /> copied
        {:else}
          <Copy class="w-3 h-3" /> copy
        {/if}
      </button>
    </div>
    <pre class="p-4 font-mono text-xs text-frost overflow-x-auto leading-relaxed"><code>{pretty}</code></pre>
  </div>
{/if}
