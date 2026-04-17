<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    action?: Snippet;
    children?: Snippet;
    class?: string;
  }

  let { title, description, action, children, class: klass = '' }: Props = $props();
</script>

<section class="animate-[rise_360ms_var(--ease-out-soft)] {klass}">
  <div class="flex items-end justify-between gap-4 mb-4">
    <div>
      <h2 class="font-mono text-sm font-semibold uppercase tracking-[0.16em] text-frost-hi">
        {title}
      </h2>
      {#if description}
        <p class="mt-1 text-sm text-graphite">{description}</p>
      {/if}
    </div>
    {#if action}
      <div class="shrink-0">{@render action()}</div>
    {/if}
  </div>
  {#if children}{@render children()}{/if}
</section>
