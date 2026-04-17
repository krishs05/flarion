<script lang="ts">
  interface Props {
    value: number; // 0..1
    tone?: 'ember' | 'cyan' | 'lime' | 'amber' | 'signal';
    class?: string;
  }

  let { value, tone = 'ember', class: klass = '' }: Props = $props();

  let clamped = $derived(Math.max(0, Math.min(1, value)));

  const tones = {
    ember: 'from-ember to-ember-soft',
    cyan: 'from-cyan-flare/70 to-cyan-flare',
    lime: 'from-lime/70 to-lime',
    amber: 'from-amber/70 to-amber',
    signal: 'from-signal/70 to-signal'
  } as const;
</script>

<div class="h-1.5 w-full rounded-full bg-wire/60 overflow-hidden {klass}">
  <div
    class="h-full rounded-full bg-gradient-to-r {tones[tone]} transition-[width] duration-500 ease-out"
    style="width: {clamped * 100}%"
  ></div>
</div>
