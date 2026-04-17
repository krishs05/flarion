<script lang="ts">
  import { settings, saveSettings, resetSettings } from '$lib/stores/settings.svelte';
  import { pokeConnection } from '$lib/stores/connection.svelte';

  let baseUrl = $state(settings.baseUrl);
  let temperature = $state(settings.defaultParams.temperature);
  let topP = $state(settings.defaultParams.topP);
  let maxTokens = $state(settings.defaultParams.maxTokens);
  let saved = $state(false);

  function handleSave() {
    const cleaned = baseUrl.trim().replace(/\/$/, '');
    settings.baseUrl = cleaned;
    settings.defaultParams = { temperature, topP, maxTokens };
    saveSettings();
    pokeConnection();
    saved = true;
    setTimeout(() => (saved = false), 1500);
  }

  function handleReset() {
    if (!confirm('reset all settings to defaults?')) return;
    resetSettings();
    baseUrl = settings.baseUrl;
    temperature = settings.defaultParams.temperature;
    topP = settings.defaultParams.topP;
    maxTokens = settings.defaultParams.maxTokens;
    pokeConnection();
  }
</script>

<div class="h-full overflow-y-auto p-6 max-w-2xl">
  <div class="space-y-6">
    <div>
      <h2 class="font-mono font-bold text-xl text-ember">settings</h2>
      <p class="font-mono text-xs text-graphite mt-1 uppercase tracking-wider">
        stored in localStorage · browser-only
      </p>
    </div>

    <div class="space-y-2">
      <label for="base-url" class="block font-mono text-xs text-graphite uppercase tracking-wider">
        flarion server url
      </label>
      <input
        id="base-url"
        type="text"
        bind:value={baseUrl}
        placeholder="http://localhost:8080"
        class="w-full bg-carbon border border-wire rounded-md px-3 py-2 font-mono text-sm text-frost
          focus:border-ember outline-none"
      />
      <p class="font-mono text-xs text-graphite">
        no trailing slash. example: <code class="text-cyan-flare">http://localhost:8080</code>
      </p>
    </div>

    <div class="space-y-4 pt-4 border-t border-wire">
      <h3 class="font-mono text-sm text-frost uppercase tracking-wider">default parameters</h3>

      <div>
        <div class="flex items-center justify-between mb-1">
          <label for="default-temp" class="font-mono text-xs text-graphite">temperature</label>
          <span class="font-mono text-xs text-cyan-flare">{temperature.toFixed(2)}</span>
        </div>
        <input
          id="default-temp"
          type="range"
          min="0"
          max="2"
          step="0.05"
          bind:value={temperature}
          class="w-full accent-ember"
        />
      </div>

      <div>
        <div class="flex items-center justify-between mb-1">
          <label for="default-top-p" class="font-mono text-xs text-graphite">top_p</label>
          <span class="font-mono text-xs text-cyan-flare">{topP.toFixed(2)}</span>
        </div>
        <input
          id="default-top-p"
          type="range"
          min="0"
          max="1"
          step="0.05"
          bind:value={topP}
          class="w-full accent-ember"
        />
      </div>

      <div>
        <label for="default-max-tokens" class="block font-mono text-xs text-graphite mb-1">
          max tokens
        </label>
        <input
          id="default-max-tokens"
          type="number"
          bind:value={maxTokens}
          min="1"
          max="32768"
          class="w-32 bg-carbon border border-wire rounded-md px-3 py-2 font-mono text-sm text-frost
            focus:border-ember outline-none"
        />
      </div>
    </div>

    <div class="flex items-center gap-3 pt-4 border-t border-wire">
      <button
        onclick={handleSave}
        class="px-4 py-2 bg-ember text-midnight font-mono text-sm rounded-md
          hover:shadow-[0_0_12px_rgba(255,107,43,0.3)] transition-shadow"
      >
        {saved ? 'saved!' : 'save'}
      </button>
      <button
        onclick={handleReset}
        class="px-4 py-2 bg-transparent border border-wire text-graphite font-mono text-sm rounded-md
          hover:text-signal hover:border-signal transition-colors"
      >
        reset to defaults
      </button>
    </div>
  </div>
</div>
