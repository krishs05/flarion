<script lang="ts">
  import HealthTab from './HealthTab.svelte';
  import ModelsTab from './ModelsTab.svelte';
  import ChatTab from './ChatTab.svelte';

  type SubTab = 'health' | 'models' | 'chat';
  let activeSubTab = $state<SubTab>('health');

  const subTabs: { id: SubTab; label: string }[] = [
    { id: 'health', label: 'health' },
    { id: 'models', label: 'models' },
    { id: 'chat', label: 'chat completions' }
  ];
</script>

<div class="h-full flex flex-col overflow-hidden">
  <div class="flex border-b border-wire bg-carbon">
    {#each subTabs as tab}
      <button
        onclick={() => (activeSubTab = tab.id)}
        class="px-4 py-2 font-mono text-xs uppercase tracking-wider transition-colors
          {activeSubTab === tab.id
            ? 'text-cyan-flare border-b-2 border-cyan-flare -mb-px'
            : 'text-graphite hover:text-frost'}"
      >
        {tab.label}
      </button>
    {/each}
  </div>

  <div class="flex-1 overflow-y-auto p-6">
    {#if activeSubTab === 'health'}
      <HealthTab />
    {:else if activeSubTab === 'models'}
      <ModelsTab />
    {:else}
      <ChatTab />
    {/if}
  </div>
</div>
