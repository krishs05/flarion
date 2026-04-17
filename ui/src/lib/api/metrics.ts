export interface MetricSample {
  name: string;
  labels: Record<string, string>;
  value: number;
}

export interface MetricFamily {
  name: string;
  help: string;
  type: string;
  samples: MetricSample[];
}

/** Minimal Prometheus text-format parser. Handles counters, gauges, histograms. */
export function parseMetrics(text: string): Map<string, MetricFamily> {
  const families = new Map<string, MetricFamily>();
  const lines = text.split('\n');

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) continue;

    if (line.startsWith('# HELP ')) {
      const [, name, ...rest] = line.slice(7).split(' ');
      const family = ensureFamily(families, name);
      family.help = rest.join(' ');
      continue;
    }
    if (line.startsWith('# TYPE ')) {
      const [, name, type] = line.slice(7).split(' ');
      const family = ensureFamily(families, name);
      family.type = type ?? 'untyped';
      continue;
    }
    if (line.startsWith('#')) continue;

    const sample = parseSample(line);
    if (!sample) continue;
    const baseName = baseFamilyName(sample.name);
    const family = ensureFamily(families, baseName);
    family.samples.push(sample);
  }
  return families;
}

function ensureFamily(map: Map<string, MetricFamily>, name: string): MetricFamily {
  let f = map.get(name);
  if (!f) {
    f = { name, help: '', type: 'untyped', samples: [] };
    map.set(name, f);
  }
  return f;
}

function baseFamilyName(name: string): string {
  if (name.endsWith('_bucket')) return name.slice(0, -7);
  if (name.endsWith('_sum')) return name.slice(0, -4);
  if (name.endsWith('_count')) return name.slice(0, -6);
  return name;
}

function parseSample(line: string): MetricSample | null {
  const braceStart = line.indexOf('{');
  const braceEnd = line.indexOf('}');
  let name: string;
  let labels: Record<string, string> = {};
  let rest: string;

  if (braceStart === -1) {
    const sp = line.indexOf(' ');
    if (sp === -1) return null;
    name = line.slice(0, sp);
    rest = line.slice(sp + 1);
  } else {
    name = line.slice(0, braceStart);
    labels = parseLabels(line.slice(braceStart + 1, braceEnd));
    rest = line.slice(braceEnd + 1).trim();
  }

  const valueStr = rest.split(' ')[0];
  const value = Number(valueStr);
  if (!Number.isFinite(value)) return null;
  return { name, labels, value };
}

function parseLabels(src: string): Record<string, string> {
  const out: Record<string, string> = {};
  let i = 0;
  while (i < src.length) {
    const eq = src.indexOf('=', i);
    if (eq === -1) break;
    const key = src.slice(i, eq).trim();
    const q1 = src.indexOf('"', eq + 1);
    if (q1 === -1) break;
    let q2 = q1 + 1;
    while (q2 < src.length) {
      if (src[q2] === '\\') q2 += 2;
      else if (src[q2] === '"') break;
      else q2 += 1;
    }
    out[key] = src.slice(q1 + 1, q2).replace(/\\"/g, '"').replace(/\\\\/g, '\\');
    const comma = src.indexOf(',', q2 + 1);
    if (comma === -1) break;
    i = comma + 1;
  }
  return out;
}

export interface MetricsSummary {
  requests: { total: number; canceled: number; by5xx: number };
  fallbacks: number;
  evictions: number;
  modelLoads: { success: number; over_budget: number; load_failed: number };
  vramBudgetMb: number | null;
  vramByModel: Array<{ model: string; gpu: string | null; mb: number }>;
  firstTokenP50: number | null;
  firstTokenP95: number | null;
}

export function summarize(families: Map<string, MetricFamily>): MetricsSummary {
  const out: MetricsSummary = {
    requests: { total: 0, canceled: 0, by5xx: 0 },
    fallbacks: 0,
    evictions: 0,
    modelLoads: { success: 0, over_budget: 0, load_failed: 0 },
    vramBudgetMb: null,
    vramByModel: [],
    firstTokenP50: null,
    firstTokenP95: null
  };

  const reqs = families.get('flarion_requests_total');
  if (reqs) {
    for (const s of reqs.samples) {
      out.requests.total += s.value;
      if (s.labels.status === 'canceled') out.requests.canceled += s.value;
      if (s.labels.status && /^5\d\d$/.test(s.labels.status)) out.requests.by5xx += s.value;
    }
  }

  const fb = families.get('flarion_fallbacks_total');
  if (fb) out.fallbacks = sumAll(fb.samples);

  const ev = families.get('flarion_model_evictions_total');
  if (ev) out.evictions = sumAll(ev.samples);

  const loads = families.get('flarion_model_loads_total');
  if (loads) {
    for (const s of loads.samples) {
      const r = s.labels.result as 'success' | 'over_budget' | 'load_failed' | undefined;
      if (r && r in out.modelLoads) out.modelLoads[r] += s.value;
    }
  }

  const budget = families.get('flarion_vram_budget_mb');
  if (budget && budget.samples.length > 0) {
    out.vramBudgetMb = budget.samples.reduce((acc, s) => acc + s.value, 0);
  }

  const reserved = families.get('flarion_vram_reserved_mb');
  if (reserved) {
    for (const s of reserved.samples) {
      if (!s.labels.model) continue;
      out.vramByModel.push({
        model: s.labels.model,
        gpu: s.labels.gpu ?? null,
        mb: s.value
      });
    }
  }

  const ft = families.get('flarion_first_token_seconds');
  if (ft) {
    const buckets = ft.samples
      .filter((s) => s.name === 'flarion_first_token_seconds_bucket' && s.labels.le)
      .sort((a, b) => Number(a.labels.le) - Number(b.labels.le));
    const totalSample = ft.samples.find((s) => s.name === 'flarion_first_token_seconds_count');
    const total = totalSample?.value ?? 0;
    if (total > 0 && buckets.length > 0) {
      out.firstTokenP50 = histogramQuantile(0.5, buckets, total);
      out.firstTokenP95 = histogramQuantile(0.95, buckets, total);
    }
  }

  return out;
}

function sumAll(samples: MetricSample[]): number {
  return samples.reduce((acc, s) => acc + s.value, 0);
}

function histogramQuantile(q: number, buckets: MetricSample[], total: number): number {
  const target = q * total;
  let prevLe = 0;
  let prevCount = 0;
  for (const b of buckets) {
    const le = Number(b.labels.le);
    if (b.value >= target) {
      if (le === Infinity) return prevLe;
      const frac = (target - prevCount) / Math.max(b.value - prevCount, 1e-9);
      return prevLe + (le - prevLe) * frac;
    }
    prevLe = le;
    prevCount = b.value;
  }
  return prevLe;
}
