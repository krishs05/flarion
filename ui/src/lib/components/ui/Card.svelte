<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    padding?: 'none' | 'sm' | 'md' | 'lg';
    hover?: boolean;
    accent?: boolean;
    class?: string;
    children?: Snippet;
  }

  let {
    padding = 'md',
    hover = false,
    accent = false,
    class: klass = '',
    children
  }: Props = $props();

  const pads = {
    none: '',
    sm: 'p-3',
    md: 'p-5',
    lg: 'p-7'
  } as const;
</script>

<div
  class="{hover ? 'card-hover' : 'card'} {accent ? 'gradient-border' : ''} {pads[padding]} {klass}"
>
  {#if children}{@render children()}{/if}
</div>
