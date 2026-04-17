<script lang="ts">
  import type { Metrics } from '$lib/stores/chats.svelte';

  interface Props {
    metrics: Metrics;
  }

  let { metrics }: Props = $props();
</script>

<div class="text-xs font-mono text-cyan-flare mt-2">
  ttft: {metrics.ttft}ms
  · {metrics.tokensPerSecond.toFixed(1)} tok/s
  · {metrics.totalTokens} tokens
  · {(metrics.durationMs / 1000).toFixed(2)}s
</div>
