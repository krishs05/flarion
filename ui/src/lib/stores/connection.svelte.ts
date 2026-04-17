import { getHealth, getMetrics, listModels } from '$lib/api/client';
import { parseMetrics, summarize, type MetricsSummary } from '$lib/api/metrics';
import type { ModelStatus } from '$lib/api/types';
import { settings } from './settings.svelte';

export interface ConnectionState {
  connected: boolean;
  version: string | null;
  models: ModelStatus[];
  modelId: string | null;
  modelLoaded: boolean;
  allHealthy: boolean;
  modelCount: number;
  loadedCount: number;
  error: string | null;
  lastCheck: number;
  metrics: MetricsSummary | null;
  metricsAvailable: boolean;
}

export const connection = $state<ConnectionState>({
  connected: false,
  version: null,
  models: [],
  modelId: null,
  modelLoaded: false,
  allHealthy: false,
  modelCount: 0,
  loadedCount: 0,
  error: null,
  lastCheck: 0,
  metrics: null,
  metricsAvailable: false
});

let pollInterval: ReturnType<typeof setInterval> | null = null;
const POLL_MS = 10_000;
const REQUEST_TIMEOUT_MS = 2_000;

async function checkOnce() {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

  try {
    await getHealth(settings.baseUrl, controller.signal);
    const listed = await listModels(settings.baseUrl, controller.signal);
    const models: ModelStatus[] = listed.data.map((m) => ({
      id: m.id,
      loaded: m.loaded
    }));
    connection.connected = true;
    connection.version = null;
    connection.models = models;
    connection.modelCount = models.length;
    connection.loadedCount = models.filter((m) => m.loaded).length;
    const first = models[0];
    connection.modelId = first?.id ?? null;
    connection.modelLoaded = first?.loaded ?? false;
    connection.allHealthy = models.length > 0 && models.every((m) => m.loaded);
    connection.error = null;
  } catch (err) {
    connection.connected = false;
    connection.version = null;
    connection.models = [];
    connection.modelId = null;
    connection.modelLoaded = false;
    connection.allHealthy = false;
    connection.modelCount = 0;
    connection.loadedCount = 0;
    connection.error = err instanceof Error ? err.message : String(err);
  } finally {
    clearTimeout(timeout);
    connection.lastCheck = Date.now();
  }

  if (connection.connected) {
    const metricsController = new AbortController();
    const metricsTimeout = setTimeout(() => metricsController.abort(), REQUEST_TIMEOUT_MS);
    try {
      const text = await getMetrics(settings.baseUrl, metricsController.signal);
      if (text) {
        connection.metrics = summarize(parseMetrics(text));
        connection.metricsAvailable = true;
      } else {
        connection.metricsAvailable = false;
      }
    } catch {
      connection.metricsAvailable = false;
    } finally {
      clearTimeout(metricsTimeout);
    }
  } else {
    connection.metrics = null;
    connection.metricsAvailable = false;
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
