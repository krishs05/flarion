# Changelog

All notable changes to Flarion. Format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer-ish pre-1.0 rules (minor = feature add, patch = fix).

## 0.11.0 — 2026-04-19

Completes the headless CLI surface and adds the branded splash banner and chat subcommand. No breaking changes from 0.10.0.

### Added

- **`flarion version`** — client + server version info with a branded splash banner rasterized from `assets/flarion-mark.svg` via `resvg` + unicode half-blocks. Gracefully falls back to an ASCII fallback (committed alongside the SVG) under `NO_COLOR`, `FLARION_ASCII=1`, or non-TTY stdout.
- **`flarion health`** — `/health` probe with latency.
- **`flarion gpu [<id>]`** — per-GPU inspection with optional id filter.
- **`flarion models list|show|load|unload|pin|unpin`** — full model-lifecycle surface, with confirmation prompts on destructive operations (skippable via `--yes`).
- **`flarion routes list|show`** — route configuration + hit counts.
- **`flarion requests tail [-n N] [--follow]`** — request log with optional SSE live streaming.
- **`flarion config show [--raw]`** and **`flarion config validate [-c path]`** — server-effective config (redacted by default) and local-file validation.
- **`flarion status --watch`** — 1 Hz refresh-in-place loop, Ctrl+C to exit.
- **`flarion chat "prompt"`** — one-shot chat, streams by default when stdout is a TTY (`--no-stream` to disable). Reads prompt from stdin with `-`.
- **`flarion chat --repl`** — interactive REPL with arrow-key history (persisted to `$XDG_DATA_HOME/flarion/chat_history` on Unix, `%LOCALAPPDATA%\flarion\chat_history` on Windows), slash commands (`/exit`, `/clear`, `/model <id>`, `/help`), and mid-stream Ctrl+C cancel.
- **`flarion completions <bash|zsh|fish|powershell>`** — shell completion script generator.

### Changed

- Endpoint resolution consolidated behind an `EndpointArgs` trait in `src/cli/resolve.rs` — no more per-command boilerplate.

### Internal

- New deps: `rustyline` (REPL input), `resvg` + `tiny-skia` + `fontdb` (SVG rasterization), `clap_complete` (shell completions).
- New client methods: `FlarionClient::chat_nonstream`, `chat_stream`, `stream_requests`.
- Baseline tests 400 → 417.

### Deferred (Phase 3)

- Full TUI dashboard (ratatui with tabs, command palette, live request log view).
- Dynamic value completion (model IDs in `flarion models unload <TAB>`).
- Light theme.

## 0.10.0 — 2026-04-18

First-class observability and control surface on top of Flarion's inference gateway.

### Added

- **Admin API at `/v1/admin/*`** (bearer-auth gated, same key as `/v1/*`):
  - `GET version`, `status`, `gpus`, `models`, `routes`, `config` (redacted), `requests?tail=N`
  - `GET requests/stream` — SSE stream of lifecycle events, with `Gap` events for lagged subscribers
  - `POST models/{id}/{load,unload,pin,unpin}` — 404 unknown, 409 busy, 200 on success
- **Request lifecycle tracking.** Every chat request emits `Started`/`Completed`/`Failed`/`Canceled` events into a 1000-entry ring buffer plus a `tokio::sync::broadcast` channel. Non-streaming and streaming paths both instrumented. Captures route/backend/matched-rule/fallback-count/duration/token counts. RAII guards keep counters balanced on panic and client disconnect.
- **Per-model in-flight counters.** Atomic per-backend, surfaced at `/admin/status` and `/admin/models`. Gates `unload` (returns 409 if in-flight > 0).
- **`flarion` subcommand dispatch.**
  - `flarion serve` — today's server (unchanged behavior, all existing flags preserved).
  - `flarion status [--url --api-key --endpoint --json --client-config]` — human or JSON snapshot.
  - `flarion endpoints list|add|remove|use|test` — manage named endpoints.
  - `flarion login <name>` — interactive first-run wizard.
  - `flarion -c file.toml` (no subcommand) — compat shim, routes to `serve` with a stderr deprecation warning.
  - Bare `flarion` — prints TUI-placeholder hint and exits with code 2.
- **Client library** (`src/cli/`): `FlarionClient` + `CachedClient` (1 s TTL + mutation invalidation) over `reqwest`. Error enum covers `Unreachable`/`Unauthorized`/`NotFound`/`Conflict`/`Timeout`/`Server`/`Decode`/`Stream`.
- **Endpoint config file.** `~/.config/flarion/config.toml` (Unix) or `%APPDATA%\flarion\config.toml` (Windows). Supports `${ENV_VAR}` interpolation and `api_key_cmd` shell-exec for password manager integration. Files are created 0600 on Unix.
- **Resolution precedence.** Flags → env vars (`FLARION_URL`, `FLARION_API_KEY`) → `--endpoint <name>` in client config → client-config `default` → local `flarion.toml` → loopback fallback `http://127.0.0.1:8080`.
- **`[admin]` config section** (additive, optional): `request_history_size` (default 1000).
- **Exit codes.** `0` success, `1` generic, `2` unauthorized, `3` unreachable, `4` not found, `5` conflict. Scripts can branch on these.
- **`CHANGELOG.md`** — you're reading it.

### Fixed

- `flarion serve` now actually mounts the admin router. The initial CLI split extracted `main()` verbatim and left the server using the non-admin `create_router`; admin routes were defined and unit-tested but 404'd against the real binary. Caught on first live walkthrough, fixed in the same release.

### Changed

- `create_router` and `create_router_with_admin` both delegate to a private `api_sub_router` helper, eliminating route-table duplication introduced earlier in the CLI branch.
- `ApiState` threads the optional admin handle through to chat request handlers. Sibling handlers (`list_models`, `health_check`) continue to extract `State<Arc<BackendRegistry>>` via `FromRef`.

### Test coverage

- Baseline 325 tests → **400 tests** (408 with `hf_cuda`), zero regressions.
- New: 11 admin API integration tests, 16 client HTTP tests (wiremock), 4 cache tests, 4 resolution tests, 9 endpoint-file tests, 3 status renderer tests, 3 CLI dispatch smoke tests, 3 chat-handler emission tests (non-streaming + streaming + no-admin).

### Deferred (Phase 2)

- Branded TUI dashboard (tabbed Overview / Models / GPUs / Routes / Requests / Chat), splash rasterized from `assets/flarion-mark.svg`.
- `flarion chat` streaming REPL.
- `flarion status --watch` and `flarion requests tail --follow`.
- Remaining read/mutation subcommands (`gpu`, `models list/show/load/unload/pin/unpin`, `routes`, `config show`).

### Upgrading

Drop-in for 0.9.x configs. No action required; admin endpoints become available automatically when you start `flarion serve`.

## 0.9.0 — Multi-GPU scheduling

See git log for detail. Explicit per-model `gpus = [0, 1, ...]` placement with tensor-parallel split, per-device VRAM budget overrides, best-fit auto-placement.

## 0.8.0 — LRU hot-swap, pinning, NVML auto-budget

## 0.7.0 — Lazy loading & VRAM budgets

## 0.6.0 — Streaming, worker-thread isolation, cancel-on-disconnect

## 0.5.0 — Secure defaults (breaking)

## 0.4.x — Cloud backends (OpenAI / Groq / Anthropic)

## Phase 1 — Multi-model config format
