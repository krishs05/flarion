<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  interface Props extends HTMLInputAttributes {
    mono?: boolean;
  }

  let {
    mono = false,
    class: klass = '',
    value = $bindable(),
    ...rest
  }: Props = $props();
</script>

<input
  bind:value
  {...rest}
  class="w-full h-9 px-3 rounded-lg bg-midnight/60 border border-wire text-frost
    placeholder:text-graphite transition-colors duration-150
    hover:border-wire-hi focus:border-ember focus:bg-midnight outline-none
    disabled:opacity-50 disabled:cursor-not-allowed
    {mono ? 'font-mono text-sm' : 'font-sans text-sm'} {klass}"
/>
