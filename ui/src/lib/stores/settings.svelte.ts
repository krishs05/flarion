const STORAGE_KEY = 'flarion.settings';

export interface DefaultParams {
  temperature: number;
  topP: number;
  maxTokens: number;
}

export interface Settings {
  baseUrl: string;
  defaultParams: DefaultParams;
}

const DEFAULT: Settings = {
  baseUrl: 'http://localhost:8080',
  defaultParams: {
    temperature: 0.7,
    topP: 0.9,
    maxTokens: 2048
  }
};

function load(): Settings {
  if (typeof localStorage === 'undefined') return DEFAULT;
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return DEFAULT;
  try {
    const parsed = JSON.parse(raw);
    return {
      baseUrl: parsed.baseUrl ?? DEFAULT.baseUrl,
      defaultParams: {
        temperature: parsed.defaultParams?.temperature ?? DEFAULT.defaultParams.temperature,
        topP: parsed.defaultParams?.topP ?? DEFAULT.defaultParams.topP,
        maxTokens: parsed.defaultParams?.maxTokens ?? DEFAULT.defaultParams.maxTokens
      }
    };
  } catch {
    return DEFAULT;
  }
}

export const settings = $state<Settings>(load());

export function saveSettings() {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

export function resetSettings() {
  settings.baseUrl = DEFAULT.baseUrl;
  settings.defaultParams = { ...DEFAULT.defaultParams };
  saveSettings();
}
