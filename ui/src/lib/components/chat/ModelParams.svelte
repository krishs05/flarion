<script lang="ts">
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import Sliders from '@lucide/svelte/icons/sliders-horizontal';
  import Check from '@lucide/svelte/icons/check';
  import { connection } from '$lib/stores/connection.svelte';
  import Input from '$lib/components/ui/Input.svelte';

  interface Props {
    model: string;
    temperature: number;
    topP: number;
    maxTokens: number;
    onChange: (p: { model: string; temperature: number; topP: number; maxTokens: number }) => void;
  }

  let { model, temperature, topP, maxTokens, onChange }: Props = $props();

  let modelOpen = $state(false);
  let paramsOpen = $state(false);

  function emit() {
    onChange({ model, temperature, topP, maxTokens });
  }

  function pickModel(id: string) {
    model = id;
    modelOpen = false;
    emit();
  }
</script>

<div class="shrink-0 px-5 py-3 border-b border-wire bg-carbon/30 backdrop-blur-sm flex items-center gap-3 flex-wrap">
  <!-- Model selector -->
  <div class="relative">
    <button
      type="button"
      onclick={() => (modelOpen = !modelOpen)}
      class="h-8 pl-3 pr-2 rounded-lg bg-surface border border-wire hover:border-wire-hi
        font-mono text-xs text-frost flex items-center gap-2 transition-colors"
    >
      <span class="w-1.5 h-1.5 rounded-full bg-ember animate-[pulse-soft_2.2s_ease-in-out_infinite]"></span>
      <span class="truncate max-w-[240px]">{model || 'select model'}</span>
      <ChevronDown class="w-3.5 h-3.5 text-graphite" />
    </button>
    {#if modelOpen}
      <div
        class="absolute top-full mt-1 left-0 z-30 min-w-[280px] glass-strong rounded-xl
          py-1.5 shadow-2xl"
      >
        {#if connection.models.length === 0}
          <div class="px-3 py-2 font-mono text-xs text-graphite">no models available</div>
        {:else}
          {#each connection.models as m (m.id)}
            <button
              type="button"
              onclick={() => pickModel(m.id)}
              class="w-full flex items-center justify-between gap-3 px-3 py-2
                hover:bg-surface/70 transition-colors text-left"
            >
              <div class="flex items-center gap-2 min-w-0">
                <span
                  class="w-1.5 h-1.5 rounded-full shrink-0 {m.loaded ? 'bg-lime' : 'bg-graphite'}"
                ></span>
                <span class="font-mono text-xs text-frost truncate">{m.id}</span>
              </div>
              {#if m.id === model}
                <Check class="w-3.5 h-3.5 text-ember shrink-0" />
              {/if}
            </button>
          {/each}
        {/if}
      </div>
    {/if}
  </div>

  <!-- Params popover -->
  <div class="relative">
    <button
      type="button"
      onclick={() => (paramsOpen = !paramsOpen)}
      class="h-8 px-3 rounded-lg bg-surface border border-wire hover:border-wire-hi
        font-mono text-xs text-graphite-hi flex items-center gap-2 transition-colors"
    >
      <Sliders class="w-3.5 h-3.5" />
      <span>T {temperature.toFixed(1)}</span>
      <span class="text-wire">·</span>
      <span>p {topP.toFixed(2)}</span>
      <span class="text-wire">·</span>
      <span>{maxTokens}</span>
    </button>
    {#if paramsOpen}
      <div
        class="absolute top-full mt-1 left-0 z-30 w-[320px] glass-strong rounded-xl p-4 space-y-3 shadow-2xl"
      >
        <label class="block">
          <div class="flex items-center justify-between font-mono text-[11px] uppercase tracking-wider text-graphite mb-1.5">
            <span>temperature</span>
            <span class="text-cyan-flare">{temperature.toFixed(2)}</span>
          </div>
          <input
            type="range" min="0" max="2" step="0.05"
            bind:value={temperature} oninput={emit}
            class="w-full"
          />
        </label>

        <label class="block">
          <div class="flex items-center justify-between font-mono text-[11px] uppercase tracking-wider text-graphite mb-1.5">
            <span>top_p</span>
            <span class="text-cyan-flare">{topP.toFixed(2)}</span>
          </div>
          <input
            type="range" min="0" max="1" step="0.01"
            bind:value={topP} oninput={emit}
            class="w-full"
          />
        </label>

        <label class="block">
          <div class="font-mono text-[11px] uppercase tracking-wider text-graphite mb-1.5">
            max tokens
          </div>
          <Input
            type="number" min="1" max="32768" mono
            bind:value={maxTokens} oninput={emit}
          />
        </label>
      </div>
    {/if}
  </div>
</div>
