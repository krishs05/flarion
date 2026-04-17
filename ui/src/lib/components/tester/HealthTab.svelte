<script lang="ts">
  import { getHealth } from '$lib/api/client';
  import { settings } from '$lib/stores/settings.svelte';
  import { FlarionApiError } from '$lib/api/types';
  import ResponseViewer from './ResponseViewer.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Play from '@lucide/svelte/icons/play';

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
      data = await getHealth(settings.baseUrl);
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
    <Badge tone="cyan">GET</Badge>
    <code class="font-mono text-sm text-frost-hi">{settings.baseUrl}/health</code>
  </div>

  <Button variant="primary" onclick={run} {loading}>
    {#snippet icon()}<Play class="w-3.5 h-3.5" />{/snippet}
    {loading ? 'testing…' : 'send request'}
  </Button>

  <ResponseViewer {data} {status} {error} />
</div>
