<p align="center">
  <img src="../assets/flarion.svg" alt="Flarion" width="320" />
</p>

<h1 align="center">Flarion UI</h1>

<p align="center">
  Minimalist, observability-first dashboard for the <a href="../README.md">Flarion</a> inference server.<br/>
  Tracks multi-model, multi-GPU workloads and streams OpenAI-compatible chat — all in a single page.
</p>

---

## Quick Start

```bash
npm install
npm run dev
```

Opens at <http://localhost:5173>. Point it at your Flarion API base URL (default in-app: <http://127.0.0.1:8080>, matching a typical `host = "127.0.0.1"` / `port = 8080` in `flarion.toml`). Change this under **Settings** if your server binds elsewhere.

## Build

```bash
npm run build
```

Outputs a pre-rendered static bundle to `build/`. Serve with any static host (nginx, Caddy, Cloudflare Pages, Vercel static, `npx serve build`, etc.).

## Layout

| View | Purpose |
| --- | --- |
| **Overview** | Cluster status hero, observability stat grid (requests, TTFT p50/p95, VRAM, evictions), per-GPU VRAM cards, model residency summary. |
| **Chat** | Streaming OpenAI-compatible chat with per-model selector, sampling popover, refined bubbles, abortable streams, persistent history. |
| **Models** | Registry with loaded / pinned / lazy chips, per-model VRAM bar, aggregate budget utilization. |
| **API Tester** | `/health`, `/v1/models`, and `/v1/chat/completions` sandboxes with editable JSON body and response viewer. |
| **Settings** | Endpoint, sampling defaults, local data management. |

## Branding

- Source mark: `src/lib/assets/flarion.svg` (full card lockup).
- Runtime mark: `src/lib/components/FlarionMark.svelte` — transparent, `currentColor`-tinted glyph used in the sidebar and inline surfaces.
- Favicon: `static/flarion.svg` — served as both `image/svg+xml` favicon and Apple touch icon.

Swap the source file to re-brand; the glyph inside `FlarionMark.svelte` is a detached copy so the full-card artwork and the inline mark can evolve independently.

## Stack

- **SvelteKit** + **Svelte 5** (runes — `$state`, `$derived`, `$effect`, `$props`, snippet children)
- **Tailwind CSS 4** with `@theme` design tokens and custom `@utility` primitives (`card`, `glass`, `ring-ember`, `gradient-border`)
- **TypeScript**
- Streaming via `fetch` + `ReadableStream` with a hand-rolled SSE parser
- Prometheus text-format parser for `/metrics` (TTFT histograms, VRAM by model/gpu, eviction counts, request totals)
- Markdown: `marked` + `highlight.js`
- Icons: `@lucide/svelte` (tree-shaken per-icon imports)
- Typography: JetBrains Mono + Geist (fontsource variable)

## Design Tokens

All tokens live in `src/app.css` under `@theme`.

- **Brand accents:** `ember`, `cyan-flare`, `lime`, `amber`, `signal`, `violet`.
- **Neutral surface ladder:** `midnight → carbon → surface → surface-hi → wire → wire-hi`.
- **Text tiers:** `graphite` and `frost`.
