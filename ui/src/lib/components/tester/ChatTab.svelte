<script lang="ts">
  import { chatCompletion } from '$lib/api/client';
  import { settings } from '$lib/stores/settings.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { FlarionApiError } from '$lib/api/types';
  import ResponseViewer from './ResponseViewer.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Play from '@lucide/svelte/icons/play';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';

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
      data = await chatCompletion(
        settings.baseUrl,
        { ...parsed, stream: false } as Parameters<typeof chatCompletion>[1]
      );
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

<div class="space-y-5">
  <div class="flex items-center gap-3 flex-wrap">
    <Badge tone="ember">POST</Badge>
    <code class="font-mono text-sm text-frost-hi">{settings.baseUrl}/v1/chat/completions</code>
  </div>

  <div>
    <div class="flex items-center justify-between mb-2">
      <label for="chat-body" class="font-mono text-[10px] uppercase tracking-wider text-graphite">
        request body · json
      </label>
      <button
        onclick={resetBody}
        class="font-mono text-[10px] uppercase tracking-wider text-graphite hover:text-ember
          transition-colors inline-flex items-center gap-1"
      >
        <RotateCcw class="w-3 h-3" />
        reset
      </button>
    </div>
    <textarea
      id="chat-body"
      bind:value={body}
      rows="14"
      class="w-full bg-midnight/60 border border-wire rounded-lg p-3 font-mono text-xs text-frost
        focus:border-ember focus:bg-midnight outline-none resize-y leading-relaxed
        transition-colors"
    ></textarea>
  </div>

  <Button variant="primary" onclick={run} {loading}>
    {#snippet icon()}<Play class="w-3.5 h-3.5" />{/snippet}
    {loading ? 'sending…' : 'send request'}
  </Button>

  <ResponseViewer {data} {status} {error} />
</div>
