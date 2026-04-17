<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import TabBar from '$lib/components/TabBar.svelte';
  import ChatView from '$lib/components/chat/ChatView.svelte';
  import TesterView from '$lib/components/tester/TesterView.svelte';
  import SettingsView from '$lib/components/settings/SettingsView.svelte';
  import { startPolling, stopPolling } from '$lib/stores/connection.svelte';

  type Tab = 'chat' | 'tester' | 'settings';
  let activeTab = $state<Tab>('chat');

  onMount(() => {
    startPolling();
    return () => stopPolling();
  });
</script>

<div class="flex h-screen bg-midnight text-frost">
  <Sidebar />

  <main class="flex-1 flex flex-col overflow-hidden">
    <TabBar active={activeTab} onSelect={(t) => (activeTab = t)} />

    <div class="flex-1 overflow-hidden">
      {#if activeTab === 'chat'}
        <ChatView />
      {:else if activeTab === 'tester'}
        <TesterView />
      {:else}
        <SettingsView />
      {/if}
    </div>
  </main>
</div>
