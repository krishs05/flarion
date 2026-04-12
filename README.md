# flarion

> One binary. Every model. Zero compromise.

A Rust-native LLM inference gateway that unifies local model serving, multi-backend routing, and production observability — purpose-built for developers and small teams who self-host AI.

## Status

**Phase 1 (Foundation)** — Single-model serving with OpenAI-compatible API.

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

## Configuration

See `flarion.toml` for a complete example.

## API

flarion exposes an OpenAI-compatible API:

- `POST /v1/chat/completions` — Chat completions (streaming + non-streaming)
- `GET /v1/models` — List loaded models
- `GET /health` — Health check

## Building with GPU Support

```bash
# CUDA (NVIDIA)
cargo build --release --features cuda
```

## License

MIT
