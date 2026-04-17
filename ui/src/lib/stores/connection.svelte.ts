import { getHealth } from '$lib/api/client';
import { settings } from './settings.svelte';

export interface ConnectionStatus {
  connected: boolean;
  version: string | null;
  modelId: string | null;
  modelLoaded: boolean;
  allHealthy: boolean;
  modelCount: number;
  error: string | null;
  lastCheck: number;
}

export const connection = $state<ConnectionStatus>({
  connected: false,
  version: null,
  modelId: null,
  modelLoaded: false,
  allHealthy: false,
  modelCount: 0,
  error: null,
  lastCheck: 0
});

let pollInterval: ReturnType<typeof setInterval> | null = null;
const POLL_MS = 10_000;
const REQUEST_TIMEOUT_MS = 2_000;

async function checkOnce() {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

  try {
    const res = await getHealth(settings.baseUrl, controller.signal);
    connection.connected = true;
    connection.version = res.version;
    connection.allHealthy = res.all_healthy;
    connection.modelCount = res.models.length;
    const first = res.models[0];
    connection.modelId = first?.id ?? null;
    connection.modelLoaded = first?.loaded ?? false;
    connection.error = null;
  } catch (err) {
    connection.connected = false;
    connection.version = null;
    connection.modelId = null;
    connection.modelLoaded = false;
    connection.allHealthy = false;
    connection.modelCount = 0;
    connection.error = err instanceof Error ? err.message : String(err);
  } finally {
    clearTimeout(timeout);
    connection.lastCheck = Date.now();
  }
}

export function startPolling() {
  if (pollInterval) return;
  checkOnce();
  pollInterval = setInterval(checkOnce, POLL_MS);
}

export function stopPolling() {
  if (pollInterval) {
    clearInterval(pollInterval);
    pollInterval = null;
  }
}

export function pokeConnection() {
  checkOnce();
}
