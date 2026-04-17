# flarion ui

Dashboard for testing and using the Flarion inference server.

## Dev

```bash
npm install
npm run dev
```

Opens at <http://localhost:5173>. Assumes Flarion is running at <http://localhost:8080> (configurable in Settings tab).

## Build

```bash
npm run build
```

Outputs static files to `build/`. Serve with any static file host.

## Stack

- SvelteKit + Svelte 5 (runes)
- TailwindCSS 4
- TypeScript
- Streaming via fetch + ReadableStream (SSE parser)
- Markdown: marked + highlight.js
