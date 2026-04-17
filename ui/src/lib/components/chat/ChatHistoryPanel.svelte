<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import Search from '@lucide/svelte/icons/search';
  import { chatStore, newChat, setActive, deleteChat } from '$lib/stores/chats.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';

  let query = $state('');

  let filtered = $derived(
    query.trim() === ''
      ? chatStore.chats
      : chatStore.chats.filter((c) =>
          c.title.toLowerCase().includes(query.toLowerCase())
        )
  );

  function handleNew() {
    newChat(connection.modelId ?? 'unknown');
  }

  function handleDelete(e: MouseEvent, id: string) {
    e.stopPropagation();
    if (confirm('Delete this chat?')) deleteChat(id);
  }

  function relativeTime(ts: number): string {
    const diff = Date.now() - ts;
    const min = 60_000;
    if (diff < min) return 'just now';
    if (diff < 60 * min) return `${Math.floor(diff / min)}m`;
    if (diff < 24 * 60 * min) return `${Math.floor(diff / (60 * min))}h`;
    return `${Math.floor(diff / (24 * 60 * min))}d`;
  }
</script>

<aside class="w-[260px] shrink-0 border-r border-wire bg-carbon/40 backdrop-blur-sm flex flex-col">
  <div class="p-3 space-y-2 border-b border-wire">
    <Button
      variant="primary"
      size="sm"
      onclick={handleNew}
      disabled={!connection.connected}
      class="w-full"
    >
      {#snippet icon()}<Plus class="w-3.5 h-3.5" />{/snippet}
      New chat
    </Button>

    <div class="relative">
      <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-graphite pointer-events-none" />
      <Input
        bind:value={query}
        placeholder="Search chats"
        class="pl-8 h-8 text-xs"
      />
    </div>
  </div>

  <div class="flex-1 overflow-y-auto">
    {#if filtered.length === 0}
      <div class="p-6 text-center">
        <MessageSquare class="w-6 h-6 text-graphite mx-auto mb-2 opacity-60" />
        <div class="font-mono text-[11px] uppercase tracking-wider text-graphite">
          {chatStore.chats.length === 0 ? 'no conversations' : 'no matches'}
        </div>
      </div>
    {:else}
      <ul class="p-2 space-y-0.5">
        {#each filtered as chat (chat.id)}
          {@const isActive = chatStore.activeId === chat.id}
          <li
            class="group relative rounded-lg transition-all duration-150
              {isActive
                ? 'bg-surface-hi border border-wire-hi'
                : 'border border-transparent hover:bg-surface/60 hover:border-wire'}"
          >
            <button
              type="button"
              onclick={() => setActive(chat.id)}
              class="w-full text-left px-2.5 py-2"
            >
              <div class="flex items-center gap-2 pr-6">
                <span
                  class="w-1 h-4 rounded-full shrink-0 {isActive ? 'bg-ember' : 'bg-transparent'}"
                ></span>
                <span
                  class="flex-1 min-w-0 text-sm truncate {isActive
                    ? 'text-frost-hi'
                    : 'text-frost'}"
                >
                  {chat.title || 'untitled'}
                </span>
              </div>
              <div class="mt-1 pl-3 flex items-center gap-2 font-mono text-[10px] text-graphite">
                <span class="truncate">{chat.model}</span>
                <span class="text-wire">·</span>
                <span>{relativeTime(chat.updatedAt)}</span>
              </div>
            </button>
            <button
              type="button"
              onclick={(e) => handleDelete(e, chat.id)}
              class="absolute top-2 right-2 opacity-0 group-hover:opacity-100
                text-graphite hover:text-signal transition-opacity"
              aria-label="Delete chat"
            >
              <Trash2 class="w-3.5 h-3.5" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</aside>
