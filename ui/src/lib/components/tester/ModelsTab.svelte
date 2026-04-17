<script lang="ts">
  import { listModels } from '$lib/api/client';
  import { settings } from '$lib/stores/settings.svelte';
  import { FlarionApiError } from '$lib/api/types';
  import ResponseViewer from './ResponseViewer.svelte';

  let loading = $state(false);
  let data = $state<unknown>(null);
  let status = $state<number | undefined>(undefined);
  let error = $state<string | null>(null);

  async function run() {
    loading = true;
    error = null;
    data = null;
    status = undefined;
    try {
      data = await listModels(settings.baseUrl);
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
    <code class="font-mono text-sm text-cyan-flare">GET</code>
    <code class="font-mono text-sm text-frost">{settings.baseUrl}/v1/models</code>
  </div>

  <button
    onclick={run}
    disabled={loading}
    class="px-4 py-2 bg-ember text-midnight font-mono text-sm rounded-md
      hover:shadow-[0_0_12px_rgba(255,107,43,0.3)] transition-shadow
      disabled:opacity-40 disabled:cursor-not-allowed"
  >
    {loading ? 'fetching...' : 'fetch'}
  </button>

  <ResponseViewer {data} {status} {error} />
</div>
