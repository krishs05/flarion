<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar, { type View } from '$lib/components/Sidebar.svelte';
  import Topbar from '$lib/components/Topbar.svelte';
  import OverviewView from '$lib/components/overview/OverviewView.svelte';
  import ChatView from '$lib/components/chat/ChatView.svelte';
  import ModelsView from '$lib/components/models/ModelsView.svelte';
  import TesterView from '$lib/components/tester/TesterView.svelte';
  import SettingsView from '$lib/components/settings/SettingsView.svelte';
  import { startPolling, stopPolling } from '$lib/stores/connection.svelte';

  let view = $state<View>('overview');

  onMount(() => {
    startPolling();
    return () => stopPolling();
  });
</script>

<div class="flex h-screen bg-midnight text-frost overflow-hidden">
  <Sidebar active={view} onSelect={(v) => (view = v)} />

  <main class="flex-1 flex flex-col min-w-0">
    <Topbar {view} />

    <div class="flex-1 min-h-0 overflow-hidden">
      {#if view === 'overview'}
        <OverviewView />
      {:else if view === 'chat'}
        <ChatView />
      {:else if view === 'models'}
        <ModelsView />
      {:else if view === 'tester'}
        <TesterView />
      {:else}
        <SettingsView />
      {/if}
    </div>
  </main>
</div>
