<script lang="ts">
  interface Props {
    temperature: number;
    topP: number;
    maxTokens: number;
    onChange: (p: { temperature: number; topP: number; maxTokens: number }) => void;
  }

  let { temperature, topP, maxTokens, onChange }: Props = $props();

  function emit() {
    onChange({ temperature, topP, maxTokens });
  }
</script>

<div class="flex items-center gap-6 px-4 py-2 border-b border-wire bg-carbon text-xs font-mono">
  <label class="flex items-center gap-2">
    <span class="text-graphite">temp</span>
    <input
      type="range"
      min="0"
      max="2"
      step="0.1"
      bind:value={temperature}
      oninput={emit}
      class="w-24 accent-ember"
    />
    <span class="text-cyan-flare w-8 text-right">{temperature.toFixed(1)}</span>
  </label>

  <label class="flex items-center gap-2">
    <span class="text-graphite">top_p</span>
    <input
      type="range"
      min="0"
      max="1"
      step="0.05"
      bind:value={topP}
      oninput={emit}
      class="w-24 accent-ember"
    />
    <span class="text-cyan-flare w-8 text-right">{topP.toFixed(2)}</span>
  </label>

  <label class="flex items-center gap-2">
    <span class="text-graphite">max_tokens</span>
    <input
      type="number"
      bind:value={maxTokens}
      oninput={emit}
      min="1"
      max="32768"
      class="w-20 bg-midnight border border-wire rounded px-2 py-1 text-frost focus:border-ember outline-none"
    />
  </label>
</div>
