<script lang="ts">
  import Activity from '@lucide/svelte/icons/activity';
  import Zap from '@lucide/svelte/icons/zap';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Boxes from '@lucide/svelte/icons/boxes';
  import TrendingDown from '@lucide/svelte/icons/trending-down';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import AlertTriangle from '@lucide/svelte/icons/alert-triangle';
  import CheckCircle2 from '@lucide/svelte/icons/check-circle-2';

  import { budgetMbForGpuLabel } from '$lib/api/metrics';
  import { connection } from '$lib/stores/connection.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import Stat from '$lib/components/ui/Stat.svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import ProgressBar from '$lib/components/ui/ProgressBar.svelte';
  import Section from '$lib/components/ui/Section.svelte';

  let metrics = $derived(connection.metrics);
  let budgetMb = $derived(metrics?.vramBudgetMb ?? 0);
  let reservedMb = $derived(
    metrics ? metrics.vramByModel.reduce((a, m) => a + m.mb, 0) : 0
  );
  let reservedPct = $derived(budgetMb > 0 ? reservedMb / budgetMb : 0);

  type VramRow = { model: string; gpu: string | null; mb: number };
  type GpuGroup = { gpu: string; models: VramRow[]; total: number };

  let gpuGroups = $derived.by<GpuGroup[]>(() => {
    const m = metrics;
    if (!m) return [];
    const groups = new Map<string, VramRow[]>();
    for (const row of m.vramByModel) {
      const key = row.gpu ?? 'auto';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(row);
    }
    return [...groups.entries()]
      .map<GpuGroup>(([gpu, models]) => ({
        gpu,
        models,
        total: models.reduce((a, r) => a + r.mb, 0)
      }))
      .sort((a, b) => a.gpu.localeCompare(b.gpu));
  });

  function fmtMs(v: number | null): string {
    if (v === null) return '–';
    return v < 1 ? `${Math.round(v * 1000)}` : `${v.toFixed(2)}`;
  }
  function fmtMsUnit(v: number | null): string {
    if (v === null) return '';
    return v < 1 ? 'ms' : 's';
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-7xl mx-auto px-8 py-8 space-y-8">
    <!-- Hero status card -->
    <Card padding="lg" accent class="relative overflow-hidden animate-[rise_360ms_var(--ease-out-soft)]">
      <div class="absolute inset-0 bg-gradient-to-br from-ember/[0.04] via-transparent to-cyan-flare/[0.04] pointer-events-none"></div>
      <div class="relative flex items-start justify-between gap-6 flex-wrap">
        <div class="space-y-2">
          <div class="flex items-center gap-3">
            {#if connection.connected}
              <span class="w-2.5 h-2.5 rounded-full bg-cyan-flare animate-[pulse-soft_2.2s_ease-in-out_infinite]"></span>
              <span class="font-mono text-[11px] uppercase tracking-[0.2em] text-cyan-flare">
                cluster online
              </span>
            {:else}
              <span class="w-2.5 h-2.5 rounded-full bg-signal"></span>
              <span class="font-mono text-[11px] uppercase tracking-[0.2em] text-signal">
                cluster offline
              </span>
            {/if}
            {#if connection.version}
              <Badge tone="neutral" size="xs">v{connection.version}</Badge>
            {/if}
          </div>
          <h1 class="font-sans text-3xl md:text-4xl font-semibold tracking-tight text-frost-hi leading-tight">
            {#if connection.connected}
              <span class="text-graphite-hi">serving</span>
              <span class="text-ember">{connection.loadedCount}</span>
              <span class="text-graphite-hi">of</span>
              <span>{connection.modelCount}</span>
              <span class="text-graphite-hi">model{connection.modelCount === 1 ? '' : 's'}</span>
            {:else}
              <span class="text-graphite-hi">awaiting</span>
              <span class="text-ember">flarion</span>
              <span class="text-graphite-hi">backend…</span>
            {/if}
          </h1>
          <p class="font-mono text-xs text-graphite tracking-wide">
            endpoint · {settings.baseUrl}
          </p>
        </div>

        <div class="flex items-center gap-2 shrink-0">
          {#if connection.allHealthy}
            <Badge tone="lime" dot>all healthy</Badge>
          {:else if connection.connected}
            <Badge tone="amber" dot>partial</Badge>
          {:else}
            <Badge tone="signal" dot>disconnected</Badge>
          {/if}
          {#if metrics && metrics.evictions > 0}
            <span
              class="inline-flex"
              title={metrics.evictionsByGpu.map((e) => `gpu ${e.gpu}: ${e.count}`).join(' · ')}
            >
              <Badge tone="violet">
                {metrics.evictions} eviction{metrics.evictions === 1 ? '' : 's'}
              </Badge>
            </span>
          {/if}
        </div>
      </div>
    </Card>

    <!-- Stat grid -->
    <Section title="Observability" description="live metrics from /metrics (Prometheus)">
      <div class="grid grid-cols-2 md:grid-cols-4 gap-4">
        <Stat
          label="Requests"
          value={metrics?.requests.total.toLocaleString() ?? '–'}
          hint={metrics ? `${metrics.requests.canceled} canceled · ${metrics.requests.by5xx} 5xx` : 'unavailable'}
          accent="cyan"
        >
          {#snippet icon()}
            <Activity class="w-4 h-4" />
          {/snippet}
        </Stat>

        <Stat
          label="TTFT p50"
          value={fmtMs(metrics?.firstTokenP50 ?? null)}
          unit={fmtMsUnit(metrics?.firstTokenP50 ?? null)}
          hint={metrics?.firstTokenP95 !== null && metrics?.firstTokenP95 !== undefined ? `p95 · ${fmtMs(metrics.firstTokenP95)}${fmtMsUnit(metrics.firstTokenP95)}` : 'no samples yet'}
          accent="ember"
        >
          {#snippet icon()}
            <Zap class="w-4 h-4" />
          {/snippet}
        </Stat>

        <Stat
          label="VRAM reserved"
          value={budgetMb > 0 ? (reservedMb / 1024).toFixed(1) : '–'}
          unit={budgetMb > 0 ? 'GiB' : ''}
          hint={budgetMb > 0 ? `of ${(budgetMb / 1024).toFixed(1)} GiB budget` : 'no budget set'}
          accent={reservedPct > 0.9 ? 'signal' : reservedPct > 0.7 ? 'amber' : 'lime'}
        >
          {#snippet icon()}
            <HardDrive class="w-4 h-4" />
          {/snippet}
        </Stat>

        <Stat
          label="Evictions"
          value={metrics?.evictions ?? '–'}
          hint={metrics ? `${metrics.fallbacks} fallbacks` : 'unavailable'}
          accent="violet"
        >
          {#snippet icon()}
            <TrendingDown class="w-4 h-4" />
          {/snippet}
        </Stat>
      </div>
    </Section>

    <!-- VRAM per GPU -->
    {#if metrics && gpuGroups.length > 0 && budgetMb > 0}
      <Section
        title="VRAM by GPU"
        description="per-device residency — phase 2h multi-gpu scheduling"
      >
        <div class="grid gap-4 md:grid-cols-2">
          {#each gpuGroups as grp (grp.gpu)}
            <Card padding="md">
              <div class="flex items-center justify-between mb-3">
                <div class="flex items-center gap-2">
                  <div class="w-8 h-8 rounded-lg bg-cyan-flare/10 border border-cyan-flare/30 flex items-center justify-center">
                    <GitBranch class="w-4 h-4 text-cyan-flare" />
                  </div>
                  <div>
                    <div class="font-mono text-[11px] uppercase tracking-wider text-graphite">gpu</div>
                    <div class="font-mono text-sm text-frost-hi">{grp.gpu}</div>
                  </div>
                </div>
                <div class="text-right">
                  <div class="font-mono text-[11px] text-graphite">
                    {grp.models.length} model{grp.models.length === 1 ? '' : 's'}
                  </div>
                  <div class="font-mono text-sm text-frost tabular-nums">
                    {(grp.total / 1024).toFixed(2)} GiB
                  </div>
                </div>
              </div>
              {@const cap =
                metrics
                  ? budgetMbForGpuLabel(grp.gpu, metrics.vramBudgetByGpu) ??
                    (metrics.vramBudgetByGpu.length === 1 ? metrics.vramBudgetByGpu[0].mb : null)
                  : null}
              <ProgressBar
                value={cap && cap > 0 ? grp.total / cap : 0}
                tone="cyan"
              />
              {#if cap && cap > 0}
                <div class="mt-1 font-mono text-[10px] text-graphite tabular-nums">
                  {(grp.total / 1024).toFixed(2)} / {(cap / 1024).toFixed(2)} GiB on device
                </div>
              {/if}
              <ul class="mt-3 space-y-1.5">
                {#each grp.models as m (`${m.model}-${m.gpu ?? 'x'}`)}
                  <li class="flex items-center justify-between font-mono text-xs">
                    <span class="text-graphite-hi truncate">{m.model}</span>
                    <span class="text-frost tabular-nums">{(m.mb / 1024).toFixed(2)} GiB</span>
                  </li>
                {/each}
              </ul>
            </Card>
          {/each}
        </div>
      </Section>
    {/if}

    <!-- Model registry summary -->
    <Section title="Models" description="registered in this flarion instance">
      {#if connection.models.length === 0}
        <Card padding="lg" class="flex items-center gap-3 text-graphite">
          <AlertTriangle class="w-5 h-5 text-amber shrink-0" />
          <span class="font-mono text-sm">no models reported — check backend config</span>
        </Card>
      {:else}
        <div class="grid gap-3 md:grid-cols-2">
          {#each connection.models as m (m.id)}
            <Card padding="md" hover>
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0">
                  <div class="font-mono text-sm text-frost-hi truncate">{m.id}</div>
                  <div class="mt-1 flex items-center gap-1.5">
                    {#if m.loaded}
                      <Badge tone="lime" size="xs" dot>loaded</Badge>
                    {:else}
                      <Badge tone="neutral" size="xs" dot>lazy</Badge>
                    {/if}
                  </div>
                </div>
                <div class="w-9 h-9 rounded-lg bg-surface-hi border border-wire flex items-center justify-center">
                  {#if m.loaded}
                    <CheckCircle2 class="w-4 h-4 text-lime" />
                  {:else}
                    <Boxes class="w-4 h-4 text-graphite" />
                  {/if}
                </div>
              </div>
            </Card>
          {/each}
        </div>
      {/if}
    </Section>

    {#if !connection.metricsAvailable && connection.connected}
      <Card padding="md" class="flex items-center gap-3">
        <AlertTriangle class="w-4 h-4 text-amber shrink-0" />
        <span class="font-mono text-xs text-graphite">
          /metrics endpoint unavailable — observability widgets are limited.
        </span>
      </Card>
    {/if}
  </div>
</div>
