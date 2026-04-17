<script lang="ts">
  import HeartPulse from '@lucide/svelte/icons/heart-pulse';
  import Boxes from '@lucide/svelte/icons/boxes';
  import MessagesSquare from '@lucide/svelte/icons/messages-square';
  import HealthTab from './HealthTab.svelte';
  import ModelsTab from './ModelsTab.svelte';
  import ChatTab from './ChatTab.svelte';

  type SubTab = 'health' | 'models' | 'chat';
  let active = $state<SubTab>('health');

  const tabs: { id: SubTab; label: string; icon: typeof HeartPulse; route: string }[] = [
    { id: 'health', label: 'Health', icon: HeartPulse, route: 'GET /health' },
    { id: 'models', label: 'Models', icon: Boxes, route: 'GET /v1/models' },
    { id: 'chat', label: 'Chat completions', icon: MessagesSquare, route: 'POST /v1/chat/completions' }
  ];
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-5xl mx-auto px-8 py-8">
    <div class="flex gap-2 p-1 rounded-xl bg-carbon/50 border border-wire w-fit mb-6">
      {#each tabs as tab (tab.id)}
        {@const Icon = tab.icon}
        {@const isActive = active === tab.id}
        <button
          type="button"
          onclick={() => (active = tab.id)}
          class="flex items-center gap-2 px-3.5 h-9 rounded-lg font-mono text-xs uppercase tracking-wider
            transition-all
            {isActive
              ? 'bg-surface-hi text-frost-hi border border-wire-hi shadow-sm'
              : 'text-graphite hover:text-frost'}"
        >
          <Icon class="w-3.5 h-3.5" />
          {tab.label}
        </button>
      {/each}
    </div>

    <div class="card p-6">
      {#if active === 'health'}
        <HealthTab />
      {:else if active === 'models'}
        <ModelsTab />
      {:else}
        <ChatTab />
      {/if}
    </div>
  </div>
</div>
