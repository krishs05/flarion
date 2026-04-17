<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props { children?: Snippet }
  let { children }: Props = $props();
</script>

<kbd
  class="inline-flex items-center justify-center min-w-5 h-5 px-1.5 rounded-md
    bg-midnight border border-wire text-graphite-hi font-mono text-[10px]"
>
  {#if children}{@render children()}{/if}
</kbd>
