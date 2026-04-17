<script lang="ts">
  import type { Metrics } from '$lib/stores/chats.svelte';

  interface Props {
    metrics: Metrics;
  }

  let { metrics }: Props = $props();
</script>

<div class="flex items-center gap-3 font-mono text-[10px] text-graphite">
  <span class="inline-flex items-center gap-1">
    <span class="uppercase tracking-wider">ttft</span>
    <span class="text-cyan-flare tabular-nums">{metrics.ttft}ms</span>
  </span>
  <span class="text-wire">·</span>
  <span class="inline-flex items-center gap-1">
    <span class="text-lime tabular-nums">{metrics.tokensPerSecond.toFixed(1)}</span>
    <span class="uppercase tracking-wider">tok/s</span>
  </span>
  <span class="text-wire">·</span>
  <span class="inline-flex items-center gap-1">
    <span class="text-frost tabular-nums">{metrics.totalTokens}</span>
    <span class="uppercase tracking-wider">tok</span>
  </span>
  <span class="text-wire">·</span>
  <span class="inline-flex items-center gap-1">
    <span class="text-graphite-hi tabular-nums">{(metrics.durationMs / 1000).toFixed(2)}s</span>
  </span>
</div>
