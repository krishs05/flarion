<script lang="ts">
  import { chatStore, newChat, setActive, deleteChat } from '$lib/stores/chats.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import ConnectionStatus from './ConnectionStatus.svelte';

  function handleNew() {
    const model = connection.modelId ?? 'unknown';
    newChat(model);
  }

  function handleDelete(e: MouseEvent, id: string) {
    e.stopPropagation();
    if (confirm('delete this chat?')) {
      deleteChat(id);
    }
  }
</script>

<aside class="w-[280px] h-screen bg-carbon border-r border-wire flex flex-col">
  <div class="p-4 border-b border-wire">
    <h1 class="font-mono font-bold text-2xl text-ember tracking-tight">flarion</h1>
    <p class="font-mono text-xs text-graphite mt-1">inference dashboard</p>
  </div>

  <div class="p-3 border-b border-wire">
    <button
      onclick={handleNew}
      disabled={!connection.connected}
      class="w-full px-3 py-2 bg-ember text-midnight font-mono text-sm rounded-md
        hover:shadow-[0_0_12px_rgba(255,107,43,0.3)] transition-shadow
        disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:shadow-none"
    >
      + new chat
    </button>
  </div>

  <div class="flex-1 overflow-y-auto">
    {#if chatStore.chats.length === 0}
      <div class="p-4 text-graphite font-mono text-xs uppercase tracking-wider">
        no conversations yet
      </div>
    {:else}
      <ul class="py-2">
        {#each chatStore.chats as chat (chat.id)}
          <li
            class="group flex items-center justify-between transition-colors
              {chatStore.activeId === chat.id
                ? 'bg-midnight border-l-2 border-ember text-frost'
                : 'text-graphite hover:text-frost hover:bg-midnight/50'}"
          >
            <button
              onclick={() => setActive(chat.id)}
              class="flex-1 text-left px-4 py-2 truncate text-sm"
            >
              {chat.title}
            </button>
            <button
              onclick={(e) => handleDelete(e, chat.id)}
              class="opacity-0 group-hover:opacity-100 text-signal hover:text-signal
                text-xs font-mono px-3 py-2"
              aria-label="delete chat"
            >
              ✕
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="p-4 border-t border-wire">
    <ConnectionStatus />
  </div>
</aside>
