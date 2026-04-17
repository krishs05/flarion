<script lang="ts">
  import type { Snippet } from 'svelte';

  type Tone = 'neutral' | 'ember' | 'cyan' | 'lime' | 'signal' | 'amber' | 'violet';

  interface Props {
    tone?: Tone;
    size?: 'xs' | 'sm';
    dot?: boolean;
    children?: Snippet;
    class?: string;
  }

  let { tone = 'neutral', size = 'sm', dot = false, children, class: klass = '' }: Props = $props();

  const tones: Record<Tone, string> = {
    neutral: 'bg-wire/60 text-graphite-hi border-wire',
    ember: 'bg-ember/10 text-ember border-ember/30',
    cyan: 'bg-cyan-flare/10 text-cyan-flare border-cyan-flare/30',
    lime: 'bg-lime/10 text-lime border-lime/30',
    signal: 'bg-signal/10 text-signal border-signal/30',
    amber: 'bg-amber/10 text-amber border-amber/30',
    violet: 'bg-violet/10 text-violet border-violet/30'
  };

  const dotColors: Record<Tone, string> = {
    neutral: 'bg-graphite',
    ember: 'bg-ember',
    cyan: 'bg-cyan-flare',
    lime: 'bg-lime',
    signal: 'bg-signal',
    amber: 'bg-amber',
    violet: 'bg-violet'
  };

  const sizes = {
    xs: 'h-5 px-1.5 text-[10px] gap-1',
    sm: 'h-6 px-2 text-[11px] gap-1.5'
  } as const;
</script>

<span
  class="inline-flex items-center font-mono uppercase tracking-wider rounded-full border {sizes[size]} {tones[tone]} {klass}"
>
  {#if dot}
    <span class="w-1.5 h-1.5 rounded-full {dotColors[tone]}"></span>
  {/if}
  {#if children}{@render children()}{/if}
</span>
