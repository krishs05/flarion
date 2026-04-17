# flarion ui

Minimalist, observability-first dashboard for the Flarion inference server.
Tracks multi-model, multi-GPU workloads and streams OpenAI-compatible chat.

## Dev

```bash
npm install
npm run dev
```

Opens at <http://localhost:5173>. Assumes Flarion is running at <http://localhost:8080>
(configurable in the **Settings** view).

## Build

```bash
npm run build
```

Outputs static files to `build/`. Serve with any static host.

## Layout

- **Overview** — cluster status hero, observability stat grid (requests, TTFT
  p50/p95, VRAM, evictions), per-GPU VRAM cards, and a model residency summary.
- **Chat** — streaming OpenAI-compatible chat with per-model selector, sampling
  popover, refined bubbles, abortable streams, and persistent history.
- **Models** — registry with loaded/pinned/lazy chips, per-model VRAM bar, and
  aggregate budget utilization (phase 2g hot-swap + 2h multi-gpu aware).
- **API Tester** — `/health`, `/v1/models`, and `/v1/chat/completions` sandboxes
  with editable JSON body and response viewer.
- **Settings** — endpoint, sampling defaults, and local data management.

## Stack

- SvelteKit + Svelte 5 (runes — `$state`, `$derived`, `$effect`, `$props`,
  snippet children)
- Tailwind CSS 4 with `@theme` design tokens and custom `@utility` primitives
  (`card`, `glass`, `ring-ember`, `gradient-border`)
- TypeScript
- Streaming via `fetch` + `ReadableStream` with a hand-rolled SSE parser
- Prometheus text-format parser for `/metrics` (TTFT histograms, VRAM by
  model/gpu, eviction counts, request totals)
- Markdown: `marked` + `highlight.js`
- Icons: `@lucide/svelte` (tree-shaken per-icon imports)
- Typography: JetBrains Mono + Geist (fontsource variable)

## Design tokens

All tokens live in `src/app.css` under `@theme`. Brand accents: `ember`,
`cyan-flare`, `lime`, `amber`, `signal`, `violet`. Neutral surface ladder:
`midnight → carbon → surface → surface-hi → wire → wire-hi`, with `graphite`
and `frost` text tiers.
