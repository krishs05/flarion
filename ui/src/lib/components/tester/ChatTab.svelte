<script lang="ts">
  import { chatCompletion } from '$lib/api/client';
  import { settings } from '$lib/stores/settings.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { FlarionApiError } from '$lib/api/types';
  import ResponseViewer from './ResponseViewer.svelte';

  function defaultBody() {
    return JSON.stringify(
      {
        model: connection.modelId ?? 'my-model',
        messages: [{ role: 'user', content: 'Say hello in one word.' }],
        temperature: 0.7,
        max_tokens: 64
      },
      null,
      2
    );
  }

  let body = $state(defaultBody());
  let loading = $state(false);
  let data = $state<unknown>(null);
  let status = $state<number | undefined>(undefined);
  let error = $state<string | null>(null);

  function resetBody() {
    body = defaultBody();
  }

  async function run() {
    loading = true;
    error = null;
    data = null;
    status = undefined;

    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(body);
    } catch (e) {
      error = 'invalid json: ' + (e instanceof Error ? e.message : String(e));
      loading = false;
      return;
    }

    try {
      data = await chatCompletion(settings.baseUrl, { ...parsed, stream: false } as Parameters<typeof chatCompletion>[1]);
      status = 200;
    } catch (e) {
      if (e instanceof FlarionApiError) {
        status = e.status;
        error = e.message;
        data = e.body;
      } else {
        error = e instanceof Error ? e.message : String(e);
      }
    } finally {
      loading = false;
    }
  }
</script>

<div class="space-y-4">
  <div class="flex items-center gap-3">
    <code class="font-mono text-sm text-cyan-flare">POST</code>
    <code class="font-mono text-sm text-frost">{settings.baseUrl}/v1/chat/completions</code>
  </div>

  <div>
    <div class="flex items-center justify-between mb-2">
      <label for="chat-body" class="font-mono text-xs text-graphite uppercase tracking-wider">
        request body (json)
      </label>
      <button
        onclick={resetBody}
        class="font-mono text-xs text-graphite hover:text-ember transition-colors"
      >
        reset
      </button>
    </div>
    <textarea
      id="chat-body"
      bind:value={body}
      rows="12"
      class="w-full bg-carbon border border-wire rounded-md p-3 font-mono text-sm text-frost
        focus:border-ember outline-none resize-y"
    ></textarea>
  </div>

  <button
    onclick={run}
    disabled={loading}
    class="px-4 py-2 bg-ember text-midnight font-mono text-sm rounded-md
      hover:shadow-[0_0_12px_rgba(255,107,43,0.3)] transition-shadow
      disabled:opacity-40 disabled:cursor-not-allowed"
  >
    {loading ? 'sending...' : 'send'}
  </button>

  <ResponseViewer {data} {status} {error} />
</div>
