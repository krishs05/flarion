<script lang="ts">
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Circle from '@lucide/svelte/icons/circle';
  import { connection, pokeConnection } from '$lib/stores/connection.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import Badge from './ui/Badge.svelte';
  import type { View } from './Sidebar.svelte';

  interface Props {
    view: View;
  }

  let { view }: Props = $props();

  const titles: Record<View, { title: string; subtitle: string }> = {
    overview: { title: 'Overview', subtitle: 'live cluster posture, budget, and throughput' },
    chat: { title: 'Chat', subtitle: 'stream tokens against a loaded model' },
    models: { title: 'Models', subtitle: 'residency, pinning, and VRAM across GPUs' },
    tester: { title: 'API Tester', subtitle: 'OpenAI-compatible endpoint sandbox' },
    settings: { title: 'Settings', subtitle: 'endpoint, defaults, and persistence' }
  };

  let meta = $derived(titles[view]);

  let spinning = $state(false);
  function refresh() {
    spinning = true;
    pokeConnection();
    setTimeout(() => (spinning = false), 600);
  }

  let budget = $derived(connection.metrics?.vramBudgetMb ?? null);
  let used = $derived(
    connection.metrics
      ? connection.metrics.vramByModel.reduce((acc, m) => acc + m.mb, 0)
      : 0
  );
  let usedPct = $derived(budget && budget > 0 ? Math.min(100, (used / budget) * 100) : 0);
</script>

<header
  class="h-14 shrink-0 flex items-center justify-between gap-6 px-6
    border-b border-wire bg-carbon/50 backdrop-blur-xl"
>
  <div class="min-w-0">
    <div class="flex items-center gap-2.5">
      <h1 class="font-mono text-[15px] font-semibold text-frost-hi tracking-tight">
        {meta.title}
      </h1>
      <span class="text-wire">/</span>
      <span class="font-mono text-[11px] uppercase tracking-[0.14em] text-graphite truncate">
        {meta.subtitle}
      </span>
    </div>
  </div>

  <div class="flex items-center gap-3 shrink-0">
    {#if connection.connected && connection.metrics && budget}
      <div
        class="hidden md:flex items-center gap-2 px-3 h-8 rounded-full
          bg-surface/70 border border-wire"
        title="VRAM reserved / budget"
      >
        <span class="font-mono text-[10px] uppercase tracking-wider text-graphite">vram</span>
        <div class="w-24 h-1 rounded-full bg-wire overflow-hidden">
          <div
            class="h-full rounded-full bg-gradient-to-r {usedPct > 90
              ? 'from-signal/70 to-signal'
              : usedPct > 70
              ? 'from-amber/70 to-amber'
              : 'from-ember/70 to-ember'} transition-[width] duration-500"
            style="width: {usedPct}%"
          ></div>
        </div>
        <span class="font-mono text-[11px] text-frost tabular-nums">
          {(used / 1024).toFixed(1)}/{(budget / 1024).toFixed(1)}
        </span>
        <span class="font-mono text-[10px] text-graphite">GiB</span>
      </div>
    {/if}

    <Badge tone={connection.connected ? 'cyan' : 'signal'} dot>
      {#if connection.connected}
        online · v{connection.version}
      {:else}
        offline
      {/if}
    </Badge>

    <span
      class="hidden sm:inline font-mono text-[10px] uppercase tracking-wider text-graphite"
      title={settings.baseUrl}
    >
      {settings.baseUrl.replace(/^https?:\/\//, '')}
    </span>

    <button
      type="button"
      onclick={refresh}
      class="w-8 h-8 rounded-lg text-graphite hover:text-frost hover:bg-surface/70
        transition-colors flex items-center justify-center"
      aria-label="Refresh status"
    >
      <RefreshCw class="w-4 h-4 {spinning ? 'animate-spin' : ''}" />
    </button>
  </div>
</header>
