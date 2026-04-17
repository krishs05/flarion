<script lang="ts">
  type Tab = 'chat' | 'tester' | 'settings';

  interface Props {
    active: Tab;
    onSelect: (tab: Tab) => void;
  }

  let { active, onSelect }: Props = $props();

  const tabs: { id: Tab; label: string }[] = [
    { id: 'chat', label: 'chat' },
    { id: 'tester', label: 'api tester' },
    { id: 'settings', label: 'settings' }
  ];
</script>

<div class="flex border-b border-wire bg-carbon">
  {#each tabs as tab}
    <button
      onclick={() => onSelect(tab.id)}
      class="px-6 py-3 font-mono text-sm uppercase tracking-wider transition-colors
        {active === tab.id
          ? 'text-ember border-b-2 border-ember -mb-px'
          : 'text-graphite hover:text-frost'}"
    >
      {tab.label}
    </button>
  {/each}
</div>
