<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    label: string;
    value: string | number;
    unit?: string;
    hint?: string;
    trend?: 'up' | 'down' | 'flat' | null;
    accent?: 'ember' | 'cyan' | 'lime' | 'amber' | 'signal' | 'violet' | 'neutral';
    icon?: Snippet;
    class?: string;
  }

  let {
    label,
    value,
    unit,
    hint,
    trend = null,
    accent = 'neutral',
    icon,
    class: klass = ''
  }: Props = $props();

  const accentColors = {
    ember: 'text-ember',
    cyan: 'text-cyan-flare',
    lime: 'text-lime',
    amber: 'text-amber',
    signal: 'text-signal',
    violet: 'text-violet',
    neutral: 'text-frost-hi'
  } as const;

  const accentBg = {
    ember: 'bg-ember/10 border-ember/30 text-ember',
    cyan: 'bg-cyan-flare/10 border-cyan-flare/30 text-cyan-flare',
    lime: 'bg-lime/10 border-lime/30 text-lime',
    amber: 'bg-amber/10 border-amber/30 text-amber',
    signal: 'bg-signal/10 border-signal/30 text-signal',
    violet: 'bg-violet/10 border-violet/30 text-violet',
    neutral: 'bg-wire/50 border-wire text-graphite-hi'
  } as const;
</script>

<div class="card p-5 group {klass}">
  <div class="flex items-start justify-between gap-2">
    <div class="font-mono text-[10px] uppercase tracking-[0.14em] text-graphite">
      {label}
    </div>
    {#if icon}
      <div class="w-8 h-8 rounded-lg border flex items-center justify-center {accentBg[accent]}">
        {@render icon()}
      </div>
    {/if}
  </div>
  <div class="mt-3 flex items-baseline gap-1.5">
    <span class="font-mono font-semibold text-[28px] tracking-tight tabular-nums {accentColors[accent]}">
      {value}
    </span>
    {#if unit}
      <span class="font-mono text-xs text-graphite">{unit}</span>
    {/if}
    {#if trend}
      <span
        class="ml-auto font-mono text-[10px] uppercase tracking-wider {
          trend === 'up' ? 'text-lime' : trend === 'down' ? 'text-signal' : 'text-graphite'
        }"
      >
        {trend === 'up' ? '▲' : trend === 'down' ? '▼' : '—'}
      </span>
    {/if}
  </div>
  {#if hint}
    <div class="mt-1 font-mono text-[11px] text-graphite">{hint}</div>
  {/if}
</div>
