<script lang="ts">
  import Cpu from '@lucide/svelte/icons/cpu';
  import Pin from '@lucide/svelte/icons/pin';
  import Layers from '@lucide/svelte/icons/layers';
  import CircleSlash from '@lucide/svelte/icons/circle-slash';

  import { connection } from '$lib/stores/connection.svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Section from '$lib/components/ui/Section.svelte';
  import ProgressBar from '$lib/components/ui/ProgressBar.svelte';

  type Row = {
    id: string;
    loaded: boolean;
    gpu: string | null;
    mb: number;
  };

  let rows = $derived.by(() => {
    const vramByModel = new Map<string, { gpu: string | null; mb: number }>();
    if (connection.metrics) {
      for (const m of connection.metrics.vramByModel) {
        vramByModel.set(m.model, { gpu: m.gpu, mb: m.mb });
      }
    }
    return connection.models.map<Row>((m) => {
      const entry = vramByModel.get(m.id);
      return {
        id: m.id,
        loaded: m.loaded,
        gpu: entry?.gpu ?? null,
        mb: entry?.mb ?? 0
      };
    });
  });

  let budgetMb = $derived(connection.metrics?.vramBudgetMb ?? 0);
  let totalMb = $derived(rows.reduce((acc, r) => acc + r.mb, 0));
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-6xl mx-auto px-8 py-8 space-y-8">
    <Section
      title="Model Registry"
      description="residency · pinning · per-device VRAM"
    >
      {#if !connection.connected}
        <Card padding="lg" class="flex items-center gap-3 text-graphite">
          <CircleSlash class="w-5 h-5 text-signal shrink-0" />
          <span class="font-mono text-sm">not connected</span>
        </Card>
      {:else if rows.length === 0}
        <Card padding="lg" class="text-graphite font-mono text-sm">
          no models registered
        </Card>
      {:else}
        <div class="space-y-3">
          {#each rows as row (row.id)}
            <Card padding="md" hover>
              <div class="flex items-center gap-4">
                <div
                  class="w-10 h-10 rounded-xl flex items-center justify-center shrink-0 border
                    {row.loaded
                      ? 'bg-ember/10 border-ember/30 text-ember'
                      : 'bg-surface-hi border-wire text-graphite'}"
                >
                  <Cpu class="w-5 h-5" />
                </div>
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="font-mono text-sm text-frost-hi truncate">{row.id}</span>
                    {#if row.loaded}
                      <Badge tone="lime" size="xs" dot>loaded</Badge>
                    {:else}
                      <Badge tone="neutral" size="xs" dot>lazy</Badge>
                    {/if}
                    {#if row.gpu}
                      <Badge tone="cyan" size="xs">gpu · {row.gpu}</Badge>
                    {/if}
                    {#if row.mb > 0}
                      <Badge tone="violet" size="xs">
                        {(row.mb / 1024).toFixed(2)} GiB
                      </Badge>
                    {/if}
                  </div>
                  {#if row.loaded && budgetMb > 0 && row.mb > 0}
                    <div class="mt-2.5">
                      <ProgressBar value={row.mb / budgetMb} tone="ember" />
                    </div>
                  {/if}
                </div>
              </div>
            </Card>
          {/each}
        </div>
      {/if}
    </Section>

    {#if budgetMb > 0}
      <Section title="Budget Utilization" description="aggregate reserved across all gpus">
        <Card padding="md">
          <div class="flex items-center justify-between mb-3 font-mono text-xs">
            <span class="text-graphite uppercase tracking-wider">reserved / budget</span>
            <span class="text-frost tabular-nums">
              {(totalMb / 1024).toFixed(2)} / {(budgetMb / 1024).toFixed(2)} GiB
            </span>
          </div>
          <ProgressBar
            value={totalMb / budgetMb}
            tone={totalMb / budgetMb > 0.9 ? 'signal' : totalMb / budgetMb > 0.7 ? 'amber' : 'ember'}
          />
        </Card>
      </Section>
    {/if}

    <Section title="Legend">
      <div class="grid gap-3 md:grid-cols-3">
        <Card padding="md" class="flex items-start gap-3">
          <Layers class="w-4 h-4 text-lime mt-0.5 shrink-0" />
          <div>
            <div class="font-mono text-xs uppercase tracking-wider text-frost-hi">loaded</div>
            <div class="text-xs text-graphite mt-1">resident in VRAM, ready to stream.</div>
          </div>
        </Card>
        <Card padding="md" class="flex items-start gap-3">
          <Pin class="w-4 h-4 text-ember mt-0.5 shrink-0" />
          <div>
            <div class="font-mono text-xs uppercase tracking-wider text-frost-hi">pinned</div>
            <div class="text-xs text-graphite mt-1">exempt from LRU hot-swap (phase 2g).</div>
          </div>
        </Card>
        <Card padding="md" class="flex items-start gap-3">
          <CircleSlash class="w-4 h-4 text-graphite mt-0.5 shrink-0" />
          <div>
            <div class="font-mono text-xs uppercase tracking-wider text-frost-hi">lazy</div>
            <div class="text-xs text-graphite mt-1">opt-in load on first request.</div>
          </div>
        </Card>
      </div>
    </Section>
  </div>
</div>
