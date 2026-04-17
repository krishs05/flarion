<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/flarion-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/flarion-light.svg">
    <img src="assets/flarion-light.svg" alt="Flarion" width="180" />
  </picture>
</p>

<h1 align="center">Flarion</h1>

<p align="center">
  <strong>One binary. Every model. Zero compromise.</strong><br/>
  A Rust-native LLM inference gateway — local GGUF serving, cloud backends, smart routing, and production observability in a single process.
</p>

<p align="center">
  <a href="#quick-start"><img alt="license" src="https://img.shields.io/badge/license-MIT-0ea5e9?style=flat-square" /></a>
  <a href="#building"><img alt="rust" src="https://img.shields.io/badge/rust-edition%202024-orange?style=flat-square&logo=rust" /></a>
  <a href="#building"><img alt="cuda" src="https://img.shields.io/badge/cuda-optional-76b900?style=flat-square&logo=nvidia" /></a>
  <a href="#api"><img alt="openai compatible" src="https://img.shields.io/badge/api-OpenAI%20compatible-111?style=flat-square" /></a>
  <a href="#metrics"><img alt="prometheus" src="https://img.shields.io/badge/metrics-Prometheus-e6522c?style=flat-square&logo=prometheus" /></a>
</p>

---

## Contents

- [Why Flarion](#why-flarion)
- [Features](#features)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Local Models & File Formats](#local-models--file-formats)
- [Cloud Backends](#cloud-backends)
- [Smart Routing](#smart-routing)
- [Dashboard UI](#dashboard-ui)
- [API](#api)
- [Streaming](#streaming)
- [Authentication](#authentication)
- [Operational Hardening](#operational-hardening)
- [Metrics](#metrics)
- [Building](#building)
- [GPU Support](#gpu-support)
- [Migration Guides](#migration-guides)
- [License](#license)

---

## Why Flarion

Most teams either run **one model in one process** (llama.cpp server, Ollama) or reach for a **full-fat inference cluster** (vLLM, TGI). Flarion sits between them: **one process** that can serve **multiple local GGUF models** *and* **upstream cloud APIs** through the **same OpenAI-compatible endpoint**, with **routing**, **metrics**, **VRAM scheduling**, and **multi-GPU placement** built in.

**Designed for:** self-hosters, internal teams, lab workstations with one or more GPUs, and anyone who wants a production-grade control plane without a Kubernetes commitment.

## Features

| Area | What you get |
| --- | --- |
| **Local inference** | llama.cpp (`llama-cpp-2`) backend with GGUF weights, full GPU offload, multi-GPU tensor split, lazy loading, LRU eviction, model pinning. |
| **Cloud backends** | OpenAI, Groq, Anthropic — `backend = "openai" \| "groq" \| "anthropic"` behind the same API, with env-var interpolation for secrets. |
| **Smart routing** | Per-request rules (prompt length, streaming flag, headers, content regex) with ordered backend fallback. |
| **Observability** | Prometheus metrics: request counters, TTFT & duration histograms, VRAM budget/reservation gauges, eviction counters, build info. |
| **Security** | Bearer token auth, SSRF guards on cloud `base_url`, CORS allow-list, opt-in plaintext upstream, dedicated metrics listener. |
| **Multi-GPU** | Explicit `gpus = [N, M, ...]` tensor split, per-GPU VRAM budgets, best-fit auto-placement. |
| **Graceful ops** | Configurable shutdown grace, in-flight drain, cancel-on-disconnect for streams, worker-thread inference isolation. |
| **Dashboard** | First-party SvelteKit UI — chat, model registry, API tester, per-GPU VRAM view. |

## Quick Start

```bash
# 1. Build (CPU-only)
cargo build --release

# 2. Configure
cp flarion.toml my-config.toml
# edit [[models]].path to point at a .gguf file

# 3. Run
./target/release/flarion -c my-config.toml
```

Then:

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"my-model","messages":[{"role":"user","content":"hello"}]}'
```

Launch the dashboard:

```bash
cd ui && npm install && npm run dev
# open http://localhost:5173 · point it at http://127.0.0.1:8080 in Settings
```

## Configuration

A minimal `my-config.toml`:

```toml
[server]
host = "127.0.0.1"
port = 8080

[[models]]
id      = "my-model"
backend = "local"
path    = "/models/my-model.gguf"
context_size = 4096
gpu_layers   = 99

[logging]
level = "info"

[metrics]
enabled = true
path    = "/metrics"
```

See `flarion.toml` for a complete, commented reference (routes, cloud backends, VRAM budgets, multi-GPU placement, per-model caps).

## Local Models & File Formats

`backend = "local"` uses **llama.cpp** under the hood and loads **GGUF** weights via `path = "…"`. Flarion does **not** natively read Hugging Face folders, `safetensors`, or PyTorch `.bin` checkpoints.

If your weights are not GGUF:

1. **Convert to GGUF** using the upstream [llama.cpp](https://github.com/ggerganov/llama.cpp) conversion and quantization tooling, then point `path` at the resulting file.
2. **Delegate inference** — run Ollama, LM Studio, vLLM, TGI, or any **OpenAI-compatible** server, and front it with a `[[models]]` entry using `backend = "openai"`, `base_url = "http://127.0.0.1:…"`, and the appropriate `upstream_model` / `api_key`.

**Long prompts.** If the formatted prompt exceeds the configured `batch_size` (default `512`), Flarion raises the effective batch size for that request up to `context_size` so the prompt fits in a single decode. No configuration change required for typical chat workloads.

## Cloud Backends

Declare cloud-hosted models alongside local ones:

```toml
[[models]]
id             = "gpt-4o"
backend        = "openai"
api_key        = "${OPENAI_API_KEY}"
upstream_model = "gpt-4o"           # optional; defaults to `id`
# base_url     = "https://openrouter.ai/api/v1"   # optional override
# timeout_secs = 300                              # optional

[[models]]
id             = "groq-llama-3.3-70b"
backend        = "groq"
api_key        = "${GROQ_API_KEY}"
upstream_model = "llama-3.3-70b-versatile"

[[models]]
id             = "claude-sonnet"
backend        = "anthropic"
api_key        = "${ANTHROPIC_API_KEY}"
upstream_model = "claude-sonnet-4-5-20250929"
```

Clients see only the local `id`. Each request's `model` field selects which configured backend serves it.

## Smart Routing

A **route** is a client-facing model id that resolves to one of several backends based on request shape, with ordered fallback on failure:

```toml
[[routes]]
id                     = "chat"
first_token_timeout_ms = 5000

  [[routes.rules]]
  name     = "long-prompt"
  matchers = { prompt_tokens_gte = 4000 }
  targets  = ["cloud-long"]

  [[routes.rules]]
  name     = "streaming-default"
  matchers = { stream = true }
  targets  = ["local-qwen", "cloud-small"]

  [[routes.rules]]
  name     = "fallback"
  matchers = {}
  targets  = ["cloud-small"]
```

**Matcher keys** (all optional, AND-combined):

| Key | Type | Description |
| --- | --- | --- |
| `stream` | bool | matches request's `stream` flag |
| `prompt_tokens_gte` / `_lte` | u32 | approximate token count (`chars / 4`) |
| `message_count_gte` / `_lte` | u32 | number of messages |
| `has_system_prompt` | bool | whether any message has `role = "system"` |
| `content_regex` | string | regex matched against the last user message |
| `header_equals` | table | exact match on request headers |

**Fallback behavior.** On retryable errors (timeout, network, HTTP 5xx/429) the router tries the next target. Non-retryable errors surface immediately. Streaming: the fallback window closes at the first chunk; mid-stream failures surface as an error.

**Response headers (non-streaming):**

- `X-Flarion-Route` — route id that served (or `direct`)
- `X-Flarion-Rule` — rule name that matched
- `X-Flarion-Backend` — the real backend that served
- `X-Flarion-Fallback-Count` — how many backends failed before this one

**Validation.** Route and model ids share one namespace; startup fails on collisions. Every route must include a catch-all (`matchers = {}`) rule.

## Dashboard UI

A SvelteKit app under `ui/` provides:

- **Overview** — cluster status, request volume, TTFT p50/p95, per-GPU VRAM, evictions
- **Chat** — streaming OpenAI-compatible chat with model selector, sampling popover, abortable streams, persistent history
- **Models** — registry with loaded / pinned / lazy chips, per-model VRAM bar, aggregate budget utilization
- **API Tester** — `/health`, `/v1/models`, and `/v1/chat/completions` sandboxes
- **Settings** — endpoint, sampling defaults, local data management

See [`ui/README.md`](ui/README.md) for run & build instructions.

## API

OpenAI-compatible surface:

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/v1/chat/completions` | Chat completions (streaming + non-streaming) |
| `GET`  | `/v1/models` | List loaded models |
| `GET`  | `/health` | Liveness check (always public) |
| `GET`  | `/metrics` | Prometheus exposition (when enabled) |

## Streaming

v0.6.0 switched to true token-by-token streaming. Chunks arrive progressively as tokens are generated, matching the OpenAI streaming contract. OpenAI SDK, LangChain, raw SSE parsers — all work unchanged.

**Cancel-on-disconnect.** If the HTTP connection drops mid-stream, Flarion aborts generation within ~1 token between decodes. Saves GPU cycles under flaky-client traffic. Canceled requests increment `flarion_requests_total{status="canceled"}`.

## Authentication

Flarion accepts unauthenticated requests by default — convenient for local development. To require auth, add keys to `[server]`:

```toml
[server]
api_keys = ["${FLARION_KEY_DEV}", "${FLARION_KEY_TEAM}"]
```

When `api_keys` is set:

- All `/v1/*` endpoints require `Authorization: Bearer <key>` matching one of the configured keys.
- `/health` stays open so monitoring / load balancers can probe.
- Missing or invalid keys return `401` with an OpenAI-style error body.

Keys are compared in constant time.

## Operational Hardening

### Authentication posture

Flarion refuses to start when all three are true:

- Server binds to a non-loopback interface (e.g. `host = "0.0.0.0"`)
- `[server].api_keys` is empty
- `[server].allow_unauthenticated` is not set to `true`

Fix by setting one of:

```toml
[server]
api_keys = ["${FLARION_KEY}"]        # preferred
# OR
allow_unauthenticated = true         # only if an upstream proxy handles auth
```

Loopback binds (`127.0.0.1`, `::1`, `localhost`) retain today's open-by-default behavior for dev UX and emit a warning at startup.

### CORS

- Empty `cors_origins` + loopback bind → permissive for dev.
- Empty list + public bind → all cross-origin requests denied.
- Explicit list wins:

  ```toml
  [server]
  cors_origins = ["https://app.example.com", "https://staging.example.com"]
  ```

### SSRF protection on cloud backends

`base_url` on `openai` / `groq` / `anthropic` entries is validated at startup. Plaintext `http://`, loopback hosts, link-local, and RFC-1918 private ranges are rejected by default. Override for legitimate dev use:

```toml
[server]
allow_plaintext_upstream = true      # enables http://, 127.0.0.1, 192.168.x.x, etc.
```

### Dedicated metrics listener

Move `/metrics` off the main listener to bypass auth for Prometheus scrapers on a trusted interface:

```toml
[metrics]
enabled = true
bind    = "127.0.0.1:9091"
```

### Per-model request caps

Each `[[models]]` entry can override the global 8192 `max_tokens` ceiling:

```toml
[[models]]
id             = "gpt-5.4-long"
backend        = "openai"
api_key        = "${OPENAI_API_KEY}"
max_tokens_cap = 16384
```

Requests above the effective cap are silently clamped.

### Graceful shutdown

On SIGTERM / Ctrl+C the server stops accepting connections and waits for in-flight inferences to finish:

```toml
[server]
shutdown_grace_secs = 30
```

- `0` — abort all in-flight; streams terminate abruptly.
- `1..=3600` — wait up to N seconds, then abandon workers and exit.
- Default `30`. Values above `3600` are clamped with a startup warning.

### Lazy loading & VRAM budgets

Mark rarely-used models with `lazy = true`; combine with `[server].vram_budget_mb` to declare more local models than fit in VRAM:

```toml
[server]
vram_budget_mb = 22000               # or "auto" (NVML, minus headroom)

[[models]]
id           = "small"
backend      = "local"
path         = "/models/qwen3-8b-q4.gguf"
gpu_layers   = 99
context_size = 8192

[[models]]
id           = "big"
backend      = "local"
path         = "/models/llama3-70b-q4.gguf"
gpu_layers   = 99
context_size = 8192
lazy         = true                  # loaded on first request
```

- First request to a lazy model pays a 3–10 s cold-start cost.
- If loading would exceed the budget, Flarion evicts the least-recently-used unpinned, non-busy model. If no candidate exists (all pinned or all busy), the request returns 503 with `Retry-After: 5`.
- VRAM footprint is estimated as `file size × 1.2` unless `vram_mb` overrides it.

## Metrics

Enable Prometheus exposition:

```toml
[metrics]
enabled = true
path    = "/metrics"
```

#### Counters

- `flarion_requests_total{route, backend, status}`
- `flarion_route_rule_matches_total{route, rule}`
- `flarion_fallbacks_total{route, from_backend, to_backend, reason}`
- `flarion_route_exhausted_total{route}`
- `flarion_model_loads_total{model, result}` — `{success, over_budget, load_failed}`
- `flarion_model_evictions_total{gpu, model, reason}`
- `flarion_model_unloads_total{model, result}`

#### Histograms

- `flarion_first_token_seconds{route, backend}`
- `flarion_request_duration_seconds{route, backend}`
- `flarion_prompt_tokens{route, backend}`
- `flarion_completion_tokens{route, backend}`

#### Gauges

- `flarion_build_info{version}`
- `flarion_vram_budget_mb{gpu}`
- `flarion_vram_reserved_mb{gpu, model}`
- `flarion_backend_poisoned{model}`

## Building

```bash
# CPU-only
cargo build --release
```

Requires Rust edition 2024. Build produces `target/release/flarion` (`.exe` on Windows).

### GPU Support

```bash
# CUDA (NVIDIA)
cargo build --release --features cuda
```

The `cuda` feature links llama.cpp against your local CUDA toolkit. Match your toolkit version to what `llama-cpp-sys` expects; see `.cargo/config.toml` for any pinned `CUDA_PATH`.

**Windows runtime:** prepend the toolkit `bin` to `PATH` so CUDA DLLs (e.g. `cublas64_*.dll`) resolve:

```powershell
$env:Path = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.2\bin;" + $env:Path
.\target\release\flarion.exe -c my-config.toml
```

**If `cargo build` fails with "Access denied" replacing `flarion.exe`:** stop the running process, or build into a separate tree:

```powershell
cargo build --release --features cuda --target-dir target/cuda-release
.\target\cuda-release\release\flarion.exe -c my-config.toml
```

## Migration Guides

### From 0.8.x → 0.9.0 — multi-GPU scheduling

All additive; single-GPU configs run unchanged.

1. **Explicit placement.** Add `gpus = [0]` (or `[1]`, etc.) to any `[[models]]` entry to pin it.
2. **Tensor-parallel split.** For models bigger than any single GPU's VRAM, use `gpus = [0, 1, 2, ...]`. Flarion invokes `llama-cpp-2` with `with_devices(&[...]) + with_split_mode(LlamaSplitMode::Layer)`.
3. **Mixed hardware.** Set `vram_budget_overrides = { 0 = N, 1 = M }` in `[server]` to give specific GPUs different budgets.
4. **Auto-placement.** Leave `gpus` unset (or `[]`) to let Flarion pick the GPU with the most free budget at first load. Decision is sticky for the model's lifetime (reset on unload).

Metric label changes: `flarion_vram_budget_mb`, `flarion_vram_reserved_mb`, and `flarion_model_evictions_total` all gained a `gpu` label.

### From 0.7.x → 0.8.0 — LRU hot-swap, pinning, NVML auto-budget

- Set `vram_budget_mb = "auto"` (+ `vram_budget_headroom_mb = 2048`) to derive the budget from NVML at startup.
- Mark always-on models with `pin = true`. Flarion refuses to start if pinned local models exceed the budget.
- When a lazy model's load would exceed the budget, Flarion now **evicts** the LRU unpinned non-busy model instead of returning 503.
- 503 `model_unavailable` now carries `Retry-After: 5`; OpenAI-compatible clients retry automatically.
- New metrics: `flarion_model_evictions_total`, `flarion_model_unloads_total`.

### From 0.6.x → 0.7.0 — lazy loading & VRAM budgets

Opt-in; default behavior identical to 0.6.x.

1. Set `[server].vram_budget_mb` to your GPU's usable VRAM.
2. Mark infrequently-used models with `lazy = true`.
3. Lazy models pay a 3–10 s cold-start on first hit.
4. Budget exceedance → 503 `model_unavailable`.

New metrics: `flarion_vram_budget_mb`, `flarion_vram_reserved_mb{model}`, `flarion_model_loads_total{model, result}`.

### From 0.5.x → 0.6.0 — streaming & worker-thread isolation

No API/config breaks, but:

- Streaming is now truly token-by-token (was batched in 0.5.x).
- New: `[server].shutdown_grace_secs` (default 30).
- New metrics: `flarion_requests_total{status="canceled"}`, `flarion_backend_poisoned{model}`.
- Internal `unsafe impl Send/Sync` for `LlamaBackend` removed in favor of a worker-thread model.

### From 0.4.x → 0.5.0 — secure defaults **(breaking)**

- Empty `api_keys` on a public bind **refuses to start**. Set `api_keys = [...]` (recommended) or `allow_unauthenticated = true`.
- `base_url` on cloud models is validated; `http://`, loopback, and private-range URLs require `allow_plaintext_upstream = true`.
- CORS is no longer permissive on public binds — configure `cors_origins`.

Loopback deployments are unaffected.

### From Phase 1 — multi-model config format

```diff
- [model]
+ [[models]]
  id = "my-model"
+ backend = "local"
  path = "/models/my-model.gguf"
  context_size = 4096
  gpu_layers = 99
```

## License

[MIT](LICENSE)
