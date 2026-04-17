<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Variant = 'primary' | 'secondary' | 'ghost' | 'danger';
  type Size = 'sm' | 'md' | 'lg';

  interface Props extends HTMLButtonAttributes {
    variant?: Variant;
    size?: Size;
    loading?: boolean;
    icon?: Snippet;
    children?: Snippet;
  }

  let {
    variant = 'secondary',
    size = 'md',
    loading = false,
    icon,
    children,
    class: klass = '',
    disabled,
    type = 'button',
    ...rest
  }: Props = $props();

  const base =
    'inline-flex items-center justify-center gap-2 font-medium rounded-lg ' +
    'transition-all duration-200 ease-out select-none whitespace-nowrap ' +
    'disabled:opacity-40 disabled:cursor-not-allowed';

  const sizes: Record<Size, string> = {
    sm: 'h-8 px-3 text-xs',
    md: 'h-9 px-4 text-sm',
    lg: 'h-11 px-5 text-[15px]'
  };

  const variants: Record<Variant, string> = {
    primary:
      'bg-ember text-midnight shadow-[0_4px_16px_-4px_rgba(255,107,43,0.55)] ' +
      'hover:bg-ember-soft hover:shadow-[0_8px_24px_-4px_rgba(255,107,43,0.6)] ' +
      'active:translate-y-[1px]',
    secondary:
      'bg-surface border border-wire text-frost ' +
      'hover:bg-surface-hi hover:border-wire-hi',
    ghost:
      'text-graphite hover:text-frost hover:bg-surface/70',
    danger:
      'bg-signal/15 border border-signal/40 text-signal hover:bg-signal/20 hover:border-signal/60'
  };
</script>

<button
  {type}
  {...rest}
  disabled={disabled || loading}
  class="{base} {sizes[size]} {variants[variant]} {klass}"
>
  {#if loading}
    <span class="w-3 h-3 rounded-full border-2 border-current border-r-transparent animate-spin"></span>
  {:else if icon}
    {@render icon()}
  {/if}
  {#if children}{@render children()}{/if}
</button>
