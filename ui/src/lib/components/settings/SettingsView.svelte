<script lang="ts">
  import Save from '@lucide/svelte/icons/save';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
  import Check from '@lucide/svelte/icons/check';
  import Server from '@lucide/svelte/icons/server';
  import Gauge from '@lucide/svelte/icons/gauge';
  import Database from '@lucide/svelte/icons/database';

  import { settings, saveSettings, resetSettings } from '$lib/stores/settings.svelte';
  import { pokeConnection, connection } from '$lib/stores/connection.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Section from '$lib/components/ui/Section.svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';

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
    if (!confirm('Reset all settings to defaults?')) return;
    resetSettings();
    baseUrl = settings.baseUrl;
    temperature = settings.defaultParams.temperature;
    topP = settings.defaultParams.topP;
    maxTokens = settings.defaultParams.maxTokens;
    pokeConnection();
  }

  function clearChats() {
    if (!confirm('Delete all chat history? This cannot be undone.')) return;
    localStorage.removeItem('flarion.chats');
    location.reload();
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-3xl mx-auto px-8 py-8 space-y-8">
    <!-- Endpoint -->
    <Section title="Endpoint" description="where the dashboard talks to flarion">
      <Card padding="md" class="space-y-3">
        <div class="flex items-center gap-3">
          <div class="w-9 h-9 rounded-lg bg-cyan-flare/10 border border-cyan-flare/30 flex items-center justify-center">
            <Server class="w-4 h-4 text-cyan-flare" />
          </div>
          <div class="flex-1 min-w-0">
            <label for="base-url" class="block font-mono text-[10px] uppercase tracking-wider text-graphite mb-1">
              flarion server url
            </label>
            <Input
              id="base-url"
              type="text"
              mono
              bind:value={baseUrl}
              placeholder="http://127.0.0.1:8080"
            />
          </div>
          <div class="shrink-0">
            {#if connection.connected}
              <Badge tone="lime" dot>online</Badge>
            {:else}
              <Badge tone="signal" dot>offline</Badge>
            {/if}
          </div>
        </div>
        <p class="font-mono text-[11px] text-graphite pl-12">
          no trailing slash · e.g. <span class="text-cyan-flare">http://127.0.0.1:8080</span>
        </p>
      </Card>
    </Section>

    <!-- Defaults -->
    <Section title="Sampling defaults" description="applied to new chats unless overridden">
      <Card padding="md" class="space-y-5">
        <div class="flex items-center gap-3">
          <div class="w-9 h-9 rounded-lg bg-ember/10 border border-ember/30 flex items-center justify-center shrink-0">
            <Gauge class="w-4 h-4 text-ember" />
          </div>
          <div class="flex-1 grid gap-4 sm:grid-cols-2">
            <div>
              <div class="flex items-center justify-between font-mono text-[10px] uppercase tracking-wider text-graphite mb-1.5">
                <span>temperature</span>
                <span class="text-cyan-flare">{temperature.toFixed(2)}</span>
              </div>
              <input type="range" min="0" max="2" step="0.05" bind:value={temperature} class="w-full" />
            </div>
            <div>
              <div class="flex items-center justify-between font-mono text-[10px] uppercase tracking-wider text-graphite mb-1.5">
                <span>top_p</span>
                <span class="text-cyan-flare">{topP.toFixed(2)}</span>
              </div>
              <input type="range" min="0" max="1" step="0.05" bind:value={topP} class="w-full" />
            </div>
            <div class="sm:col-span-2">
              <label for="default-max-tokens" class="block font-mono text-[10px] uppercase tracking-wider text-graphite mb-1.5">
                max tokens
              </label>
              <Input
                id="default-max-tokens"
                type="number"
                mono
                bind:value={maxTokens}
                min="1"
                max="32768"
                class="max-w-[180px]"
              />
            </div>
          </div>
        </div>
      </Card>
    </Section>

    <!-- Actions -->
    <div class="flex items-center gap-3">
      <Button variant="primary" onclick={handleSave}>
        {#snippet icon()}
          {#if saved}<Check class="w-3.5 h-3.5" />{:else}<Save class="w-3.5 h-3.5" />{/if}
        {/snippet}
        {saved ? 'saved' : 'save changes'}
      </Button>
      <Button variant="secondary" onclick={handleReset}>
        {#snippet icon()}<RotateCcw class="w-3.5 h-3.5" />{/snippet}
        reset defaults
      </Button>
    </div>

    <!-- Data -->
    <Section title="Local data" description="everything lives in this browser">
      <Card padding="md" class="flex items-center gap-3">
        <div class="w-9 h-9 rounded-lg bg-violet/10 border border-violet/30 flex items-center justify-center shrink-0">
          <Database class="w-4 h-4 text-violet" />
        </div>
        <div class="flex-1 min-w-0">
          <div class="font-mono text-sm text-frost-hi">Chat history</div>
          <div class="font-mono text-[11px] text-graphite mt-0.5">
            persisted in localStorage · flarion.chats
          </div>
        </div>
        <Button variant="danger" size="sm" onclick={clearChats}>
          clear history
        </Button>
      </Card>
    </Section>
  </div>
</div>
