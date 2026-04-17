<script lang="ts">
  import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import Cpu from '@lucide/svelte/icons/cpu';
  import Beaker from '@lucide/svelte/icons/beaker';
  import Settings from '@lucide/svelte/icons/settings';
  import FlarionMark from './FlarionMark.svelte';
  import { connection } from '$lib/stores/connection.svelte';

  export type View = 'overview' | 'chat' | 'models' | 'tester' | 'settings';

  interface Props {
    active: View;
    onSelect: (v: View) => void;
  }

  let { active, onSelect }: Props = $props();

  const items: { id: View; label: string; icon: typeof LayoutDashboard; badge?: () => string | null }[] = [
    { id: 'overview', label: 'Overview', icon: LayoutDashboard },
    { id: 'chat', label: 'Chat', icon: MessageSquare },
    {
      id: 'models',
      label: 'Models',
      icon: Cpu,
      badge: () =>
        connection.connected
          ? `${connection.loadedCount}/${connection.modelCount}`
          : null
    },
    { id: 'tester', label: 'API Tester', icon: Beaker },
    { id: 'settings', label: 'Settings', icon: Settings }
  ];
</script>

<aside
  class="w-[72px] h-screen bg-carbon/70 backdrop-blur-xl border-r border-wire
    flex flex-col items-center py-4 gap-1 shrink-0"
>
  <a
    href="/"
    class="w-11 h-11 rounded-xl bg-ember/10 border border-ember/30 flex items-center justify-center mb-2 ring-ember text-ember hover:bg-ember/15 transition-colors"
    aria-label="flarion"
    title="flarion"
  >
    <FlarionMark class="w-6 h-6" />
  </a>

  <nav class="flex-1 flex flex-col items-center gap-1 mt-2">
    {#each items as item (item.id)}
      {@const Icon = item.icon}
      {@const isActive = active === item.id}
      {@const badge = item.badge?.()}
      <button
        type="button"
        onclick={() => onSelect(item.id)}
        class="group relative w-11 h-11 rounded-xl flex items-center justify-center
          transition-all duration-200 ease-out
          {isActive
            ? 'bg-surface-hi text-frost-hi border border-wire-hi'
            : 'text-graphite hover:text-frost hover:bg-surface/60'}"
        aria-label={item.label}
      >
        <Icon class="w-5 h-5" />
        {#if isActive}
          <span
            class="absolute left-0 top-2 bottom-2 w-0.5 rounded-r-full bg-ember"
            aria-hidden="true"
          ></span>
        {/if}

        {#if badge}
          <span
            class="absolute -top-0.5 -right-0.5 min-w-[18px] h-[18px] px-1
              rounded-full bg-ember text-midnight font-mono text-[9px] font-semibold
              flex items-center justify-center leading-none"
          >
            {badge}
          </span>
        {/if}

        <span
          class="pointer-events-none absolute left-full ml-3 px-2.5 py-1.5 rounded-md
            bg-surface-hi border border-wire-hi text-frost text-xs font-mono whitespace-nowrap
            opacity-0 translate-x-[-4px] group-hover:opacity-100 group-hover:translate-x-0
            transition-all duration-150 z-50 shadow-lg"
        >
          {item.label}
        </span>
      </button>
    {/each}
  </nav>

  <div class="mt-auto pt-2 border-t border-wire w-10 flex justify-center">
    <span
      class="w-2.5 h-2.5 rounded-full {connection.connected
        ? 'bg-cyan-flare animate-[pulse-soft_2.2s_ease-in-out_infinite]'
        : 'bg-signal'}"
      title={connection.connected ? `online · v${connection.version}` : 'offline'}
    ></span>
  </div>
</aside>
