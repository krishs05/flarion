# flarion

> One binary. Every model. Zero compromise.

A Rust-native LLM inference gateway that unifies local model serving, multi-backend routing, and production observability — purpose-built for developers and small teams who self-host AI.

## Status

**Phase 2f** — Opt-in lazy loading and VRAM budget enforcement for local models. Previous milestones: Phase 1 foundation, Phase 2a multi-model registry, Phase 2b cloud backends, Phase 2c smart routing + metrics, Phase 2d security hardening, Phase 2e worker-thread inference isolation.

## Quick Start

```bash
# Build from source
cargo build --release

# Create a config file
cp flarion.toml my-config.toml
# Edit my-config.toml with your model path

# Run
./target/release/flarion -c my-config.toml
```

## Upgrading from Phase 1

The config format changed in v0.2.0 to support multiple models. Rewrite a single-model config:

```diff
- [model]
+ [[models]]
  id = "my-model"
+ backend = "local"
  path = "/models/my-model.gguf"
  context_size = 4096
  gpu_layers = 99
```

You can now declare additional `[[models]]` entries to serve multiple models from the same instance. Request routing uses the `model` field in chat completion requests to pick a backend; unknown model ids return 404 with the list of available models.

## Configuration

See `flarion.toml` for a complete example.

## API

flarion exposes an OpenAI-compatible API:

- `POST /v1/chat/completions` — Chat completions (streaming + non-streaming)
- `GET /v1/models` — List loaded models
- `GET /health` — Health check

## Streaming

v0.6.0 switched to true token-by-token streaming. Chunks arrive
progressively as tokens are generated, matching the OpenAI streaming
contract. Most clients (OpenAI SDK, LangChain, raw SSE parsers) handle
this natively — no client-side changes needed.

Client-disconnect cancellation: if the HTTP connection drops mid-stream,
Flarion observes the disconnect between token decodes and aborts
generation within ~1 token. Saves GPU cycles under flaky-client traffic.
Requests canceled this way count under
`flarion_requests_total{status="canceled"}`.

## Cloud Backends

flarion can serve OpenAI, Groq, and Anthropic models behind the same OpenAI-compatible API as your local models. Add a `[[models]]` entry with `backend = "openai" | "groq" | "anthropic"` and an `api_key`. Use `${VAR}` env var interpolation to keep secrets out of `flarion.toml`:

```toml
[[models]]
id = "gpt-4o"
backend = "openai"
api_key = "${OPENAI_API_KEY}"
```

Optional fields per cloud entry:
- `upstream_model` — model id sent to the upstream provider (defaults to the local `id`)
- `base_url` — override the provider's default endpoint (useful for proxies, OpenRouter, etc.)
- `timeout_secs` — request timeout in seconds (default 300)

Clients see only the local `id`, regardless of what's running upstream. Each request's `model` field selects which configured backend handles it.

## Authentication

By default, the flarion server accepts unauthenticated requests — convenient for local development. To require authentication, add `api_keys` to `[server]`:

```toml
[server]
api_keys = ["${FLARION_KEY_DEV}", "${FLARION_KEY_TEAM}"]
```

When `api_keys` is set:
- All `/v1/*` endpoints require `Authorization: Bearer <key>` matching one of the configured keys
- `/health` remains open (so monitoring / load balancers can check it)
- Missing or invalid keys return `401` with an OpenAI-style error body

Keys are compared in constant time. Use multiple keys to distribute different values to different teammates without sharing the same secret.

## Smart Routing

Routes let you address a single client-facing model id that resolves to one of several backends based on request shape, with ordered fallback on failure. Clients send `model: "chat"` and the router picks the real backend.

```toml
[[routes]]
id = "chat"
first_token_timeout_ms = 5000

  [[routes.rules]]
  name = "long-prompt"
  matchers = { prompt_tokens_gte = 4000 }
  targets = ["cloud-long"]

  [[routes.rules]]
  name = "fallback"
  matchers = {}
  targets = ["cloud-small"]
```

**Matcher keys** (all optional, AND-combined):

| Key | Type | Description |
|---|---|---|
| `stream` | bool | matches request's `stream` flag |
| `prompt_tokens_gte` / `_lte` | u32 | approximate token count (`chars / 4`) |
| `message_count_gte` / `_lte` | u32 | number of messages |
| `has_system_prompt` | bool | whether any message has `role = "system"` |
| `content_regex` | string | regex matched against last user message |
| `header_equals` | table | exact match on request headers |

**Fallback behavior:** if a target fails with a retryable error (timeout, network, HTTP 5xx, HTTP 429) the router tries the next target in the chain. Non-retryable errors (4xx) surface immediately. For streaming, the fallback window closes at the first chunk — mid-stream failures become an error to the client with no retry.

**Validation:** routes and model ids share one namespace; startup fails on collisions. Every route must have a catch-all rule (`matchers = {}`).

**Response headers** (non-streaming):
- `X-Flarion-Route` — route id that served (or `direct`)
- `X-Flarion-Rule` — rule name that matched
- `X-Flarion-Backend` — the real backend that served
- `X-Flarion-Fallback-Count` — how many backends failed before this one

## Metrics

flarion exports Prometheus-format metrics when enabled:

```toml
[metrics]
enabled = true
path = "/metrics"
```

**Counters**

- `flarion_requests_total{route, backend, status}`
- `flarion_route_rule_matches_total{route, rule}`
- `flarion_fallbacks_total{route, from_backend, to_backend, reason}`
- `flarion_route_exhausted_total{route}`
- `flarion_model_loads_total{model, result}` — model load attempts (success, over_budget, load_failed)

**Histograms**

- `flarion_first_token_seconds{route, backend}`
- `flarion_request_duration_seconds{route, backend}`
- `flarion_prompt_tokens{route, backend}`
- `flarion_completion_tokens{route, backend}`

**Gauges**

- `flarion_build_info{version}`
- `flarion_vram_budget_mb` — configured VRAM budget (set once at startup)
- `flarion_vram_reserved_mb{model}` — current reserved VRAM per model

## Operational Hardening

Flarion defaults are tuned for local development. Before exposing a Flarion
instance beyond localhost, review these settings.

### Authentication posture

Flarion refuses to start when all three are true:
- Server binds to a non-loopback interface (e.g. `host = "0.0.0.0"`)
- `[server].api_keys` is empty
- `[server].allow_unauthenticated` is not set to `true`

Fix by setting one of:

```toml
[server]
api_keys = ["${FLARION_KEY}"]       # preferred
# OR
allow_unauthenticated = true        # only if an upstream proxy handles auth
```

Loopback binds (`127.0.0.1`, `::1`, `localhost`) retain today's open-by-default
behavior for dev UX and emit a warning log at startup.

### CORS

Empty `cors_origins` + loopback bind = permissive for dev. Empty list +
public bind = all cross-origin requests denied. Configure an explicit
allow-list when serving a browser UI from a different origin:

```toml
[server]
cors_origins = ["https://app.example.com", "https://staging.example.com"]
```

### SSRF protection on cloud backends

`base_url` on `openai`/`groq`/`anthropic` entries is validated at startup.
By default, plaintext `http://`, loopback hosts, link-local, and RFC-1918
private ranges are rejected. Override for legitimate uses (e.g. pointing at
a local proxy during development):

```toml
[server]
allow_plaintext_upstream = true     # enables http://, 127.0.0.1, 192.168.x.x, etc.
```

### Dedicated metrics listener

When Prometheus scraping needs to bypass the auth layer, move `/metrics` to
a dedicated listener bound to a trusted interface:

```toml
[metrics]
enabled = true
bind = "127.0.0.1:9091"             # scraper runs on the same host
```

The main listener no longer serves `/metrics`; the dedicated listener has
no auth layer.

### Per-model request caps

Each `[[models]]` entry can override the global 8192 `max_tokens` ceiling:

```toml
[[models]]
id = "gpt-5.4-long"
backend = "openai"
api_key = "${OPENAI_API_KEY}"
max_tokens_cap = 16384
```

Requests above the effective cap are silently clamped.

### Graceful shutdown

On SIGTERM/Ctrl+C the server stops accepting new connections and waits for
in-flight inferences to finish before exiting. The grace budget is
configurable:

```toml
[server]
shutdown_grace_secs = 30
```

- `0` — abort all in-flight immediately; streams terminate abruptly, process
  exits within a few hundred milliseconds.
- `1..=3600` — wait up to N seconds for inferences to finish; after the
  budget, worker threads are abandoned and the process exits anyway.
- Default: `30`. Values above `3600` are clamped with a startup warning.

### Lazy loading

Mark rarely-used models with `lazy = true` to defer their load until the
first request. Combined with `[server].vram_budget_mb`, this lets you
declare multiple local models whose combined footprint exceeds VRAM, as
long as you don't load them all at once.

```toml
[server]
vram_budget_mb = 22000   # enforce against this budget

[[models]]
id = "small"
backend = "local"
path = "/models/qwen3-8b-q4.gguf"
gpu_layers = 99
context_size = 8192
# Loaded eagerly at startup (default).

[[models]]
id = "big"
backend = "local"
path = "/models/llama3-70b-q4.gguf"
gpu_layers = 99
context_size = 8192
lazy = true              # loaded on first request
```

- First request to a lazy model pays a 3-10s cold-start cost; subsequent
  requests are fast.
- If loading would exceed `vram_budget_mb`, the request returns 503
  `model_unavailable`. Phase 2g (future release) adds LRU eviction; for
  0.7.0 you remove/restart to free budget.
- Budget is advisory — Flarion refuses based on the configured value
  and the estimated footprint (`file size * 1.2`, or `vram_mb` override).
  Actual CUDA allocations might differ; set conservatively.

## Migrating from 0.8.x

v0.9.0 adds multi-GPU scheduling — explicit placement, tensor-parallel
split, and best-fit auto-placement. All changes are additive; single-GPU
configs run unchanged.

To use:

1. **Explicit placement.** Add `gpus = [0]` (or `[1]`, etc.) to any
   `[[models]]` entry to pin it to a specific GPU.
2. **Tensor-parallel split.** For models bigger than any single GPU's
   VRAM, use `gpus = [0, 1, 2, ...]`. Flarion invokes llama-cpp-2 with
   `with_devices(&[...]) + with_split_mode(LlamaSplitMode::Layer)`;
   llama-cpp-2 distributes the model's layers across the listed devices
   internally.
3. **Mixed hardware.** Set `vram_budget_overrides = { 0 = N, 1 = M }`
   in `[server]` to give specific GPUs different budgets.
4. **Auto-placement.** Leave `gpus` unset (or `= []`) to let Flarion
   pick the GPU with most free budget at first load. Decision is sticky
   for the model's lifetime (reset when the model unloads).

Metric label changes:
- `flarion_vram_budget_mb{gpu}` — gained `gpu` label
- `flarion_vram_reserved_mb{gpu, model}` — gained `gpu` label
- `flarion_model_evictions_total{gpu, model, reason}` — gained `gpu` label

## Migrating from 0.7.x

v0.8.0 adds LRU hot-swap, model pinning, and NVML-based auto VRAM detection.
All changes are opt-in; default behavior is identical to 0.7.x.

- Set `vram_budget_mb = "auto"` (+ `vram_budget_headroom_mb = 2048`) to let
  Flarion pick the budget from NVML at startup.
- Mark always-on models with `pin = true`. Flarion refuses to start if pinned
  local models total more than the budget.
- When a lazy model's load would exceed the budget, Flarion now evicts the
  least-recently-used unpinned non-busy model instead of returning 503. If
  no eviction candidate is available (all pinned or all busy), the request
  returns 503 with `Retry-After: 5`.
- 503 `model_unavailable` responses now carry `Retry-After: 5`;
  OpenAI-compatible clients that honor the header retry automatically.

New metrics:
- `flarion_model_evictions_total{model, reason}`
- `flarion_model_unloads_total{model, result}`

## Migrating from 0.6.x

v0.7.0 adds lazy loading and VRAM budget scheduling. All changes are
opt-in; default behavior is identical to 0.6.x.

To use:

1. Set `[server].vram_budget_mb` to your GPU's usable VRAM (MB).
2. Mark infrequently-used models with `lazy = true`.
3. Requests to lazy models pay a 3-10s cold-start cost on first hit,
   then behave identically to eager models.
4. If a load would exceed budget, the request returns 503
   `model_unavailable`. Phase 2g (next release) adds LRU eviction; for
   0.7.0 you manage loaded models by removing entries and restarting.

New metrics:

- `flarion_vram_budget_mb` — configured budget (gauge)
- `flarion_vram_reserved_mb{model}` — per-model reservation (gauge)
- `flarion_model_loads_total{model, result}` — counter with result in
  {success, over_budget, load_failed}

## Migrating from 0.5.x

v0.6.0 is a minor-but-notable release. No API or config breaks, but:

- Streaming is now truly token-by-token (was batched-and-flushed in
  0.5.x). If you had client code that relied on all chunks arriving at
  once, adjust to handle the standard progressive-chunk pattern.
- New config knob `[server].shutdown_grace_secs` (default 30).
- New metrics: `flarion_requests_total{status="canceled"}` and
  `flarion_backend_poisoned{model}` gauge.
- The internal `unsafe impl Send/Sync` for `LlamaBackend` is gone —
  worker-thread model. No externally visible change, but if you were
  vendoring the crate this is a notable soundness improvement.

## Migrating from 0.4.x

v0.5.0 is a **breaking release** on the auth default.

Before (0.4.x): empty `api_keys` meant "accept all requests, on any bind".

After (0.5.0): empty `api_keys` on a public bind refuses to start.

If you run Flarion on `0.0.0.0` or a public hostname without `api_keys`, you
have two options:

1. Set `api_keys = [...]` (recommended).
2. Set `[server].allow_unauthenticated = true` (opt-in if an upstream proxy
   handles authentication).

Loopback deployments are unaffected.

Other 0.5.0 changes:
- `base_url` on cloud models is now validated — `http://`, loopback, and
  private-range URLs require `[server].allow_plaintext_upstream = true`.
- CORS is no longer permissive on public binds by default — configure
  `[server].cors_origins` to allow browser clients.

## Building with GPU Support

```bash
# CUDA (NVIDIA)
cargo build --release --features cuda
```

## License

MIT
