<script lang="ts">
  import { connection } from '$lib/stores/connection.svelte';
</script>

<div class="flex items-center gap-2 text-xs font-mono" title={connection.error ?? ''}>
  {#if connection.connected}
    <span class="w-2 h-2 rounded-full bg-cyan-flare animate-pulse"></span>
    <span class="text-graphite">
      online · v{connection.version}
      {#if connection.modelCount === 1}
        · {connection.modelId}
      {:else}
        · {connection.modelCount} models
      {/if}
    </span>
  {:else}
    <span class="w-2 h-2 rounded-full bg-signal"></span>
    <span class="text-signal">offline</span>
  {/if}
</div>
