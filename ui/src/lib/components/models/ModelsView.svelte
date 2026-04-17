<script lang="ts">
  import Cpu from '@lucide/svelte/icons/cpu';
  import Pin from '@lucide/svelte/icons/pin';
  import Layers from '@lucide/svelte/icons/layers';
  import CircleSlash from '@lucide/svelte/icons/circle-slash';

  import { sumBudgetForGpuLabels } from '$lib/api/metrics';
  import { connection } from '$lib/stores/connection.svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Section from '$lib/components/ui/Section.svelte';
  import ProgressBar from '$lib/components/ui/ProgressBar.svelte';

  type Row = {
    id: string;
    loaded: boolean;
    gpuKeys: string[];
    totalMb: number;
    budgetForBar: number;
  };

  let rows = $derived.by(() => {
    const metrics = connection.metrics;
    const byModel = new Map<string, Array<{ gpu: string | null; mb: number }>>();
    if (metrics) {
      for (const v of metrics.vramByModel) {
        if (!byModel.has(v.model)) byModel.set(v.model, []);
        byModel.get(v.model)!.push({ gpu: v.gpu, mb: v.mb });
      }
    }
    return connection.models.map<Row>((m) => {
      const segments = byModel.get(m.id) ?? [];
      const totalMb = segments.reduce((a, s) => a + s.mb, 0);
      const gpuKeys = [
        ...new Set(
          segments.map((s) => s.gpu).filter((g): g is string => g != null && g !== '')
        )
      ].sort();
      const budgets = metrics?.vramBudgetByGpu ?? [];
      const budgetForBar =
        budgets.length > 0 ? sumBudgetForGpuLabels(gpuKeys, budgets) : metrics?.vramBudgetMb ?? 0;
      return {
        id: m.id,
        loaded: m.loaded,
        gpuKeys,
        totalMb,
        budgetForBar
      };
    });
  });

  let budgetMb = $derived(connection.metrics?.vramBudgetMb ?? 0);
  let totalMb = $derived(rows.reduce((acc, r) => acc + r.totalMb, 0));
  let budgetByGpu = $derived(connection.metrics?.vramBudgetByGpu ?? []);
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-6xl mx-auto px-8 py-8 space-y-8">
    <Section
      title="Model Registry"
      description="residency · per-GPU VRAM (tensor-parallel sums across devices)"
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
                    {#if row.gpuKeys.length > 0}
                      <Badge tone="cyan" size="xs">
                        gpu{row.gpuKeys.length > 1 ? 's' : ''} · {row.gpuKeys.join(', ')}
                      </Badge>
                    {/if}
                    {#if row.totalMb > 0}
                      <Badge tone="violet" size="xs">
                        {(row.totalMb / 1024).toFixed(2)} GiB
                      </Badge>
                    {/if}
                  </div>
                  {#if row.loaded && row.budgetForBar > 0 && row.totalMb > 0}
                    <div class="mt-2.5">
                      <ProgressBar value={row.totalMb / row.budgetForBar} tone="ember" />
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
      <Section
        title="Budget Utilization"
        description="Σ reserved / Σ per-GPU budgets (phase 2h — multi-GPU aware)"
      >
        <Card padding="md">
          <div class="flex items-center justify-between mb-3 font-mono text-xs">
            <span class="text-graphite uppercase tracking-wider">cluster reserved / capacity</span>
            <span class="text-frost tabular-nums">
              {(totalMb / 1024).toFixed(2)} / {(budgetMb / 1024).toFixed(2)} GiB
            </span>
          </div>
          <ProgressBar
            value={totalMb / budgetMb}
            tone={totalMb / budgetMb > 0.9 ? 'signal' : totalMb / budgetMb > 0.7 ? 'amber' : 'ember'}
          />
        </Card>
        {#if budgetByGpu.length > 1}
          <div class="mt-4 grid gap-3 sm:grid-cols-2">
            {#each budgetByGpu as b (b.gpu)}
              {@const used = connection.metrics
                ? connection.metrics.vramByModel
                    .filter((v) => v.gpu === b.gpu)
                    .reduce((acc, v) => acc + v.mb, 0)
                : 0}
              <Card padding="md">
                <div class="flex items-center justify-between mb-2 font-mono text-[11px] uppercase tracking-wider text-graphite">
                  <span>gpu {b.gpu}</span>
                  <span class="text-frost tabular-nums normal-case">
                    {(used / 1024).toFixed(2)} / {(b.mb / 1024).toFixed(2)} GiB
                  </span>
                </div>
                <ProgressBar
                  value={b.mb > 0 ? used / b.mb : 0}
                  tone={used / b.mb > 0.9 ? 'signal' : used / b.mb > 0.7 ? 'amber' : 'cyan'}
                />
              </Card>
            {/each}
          </div>
        {/if}
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
