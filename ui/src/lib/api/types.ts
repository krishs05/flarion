export interface ChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

export interface ChatCompletionRequest {
  model: string;
  messages: ChatMessage[];
  stream?: boolean;
  temperature?: number;
  top_p?: number;
  max_tokens?: number;
  stop?: string[];
  seed?: number;
}

export interface ChatCompletionChoice {
  index: number;
  message: ChatMessage;
  finish_reason: string;
}

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface ChatCompletionResponse {
  id: string;
  object: 'chat.completion';
  created: number;
  model: string;
  choices: ChatCompletionChoice[];
  usage: Usage;
}

export interface ChatCompletionChunkChoice {
  index: number;
  delta: { role?: string; content?: string };
  finish_reason: string | null;
}

export interface ChatCompletionChunk {
  id: string;
  object: 'chat.completion.chunk';
  created: number;
  model: string;
  choices: ChatCompletionChunkChoice[];
}

export interface ModelStatus {
  id: string;
  loaded: boolean;
}

export interface HealthResponse {
  status: string;
  version: string;
  models: ModelStatus[];
  all_healthy: boolean;
}

export interface ModelObject {
  id: string;
  object: string;
  created: number;
  owned_by: string;
}

export interface ModelsResponse {
  object: 'list';
  data: ModelObject[];
}

export interface ApiErrorBody {
  error: {
    message: string;
    type: string;
    code: string;
  };
}

export class FlarionApiError extends Error {
  constructor(
    public status: number,
    public body: ApiErrorBody | string,
    message: string
  ) {
    super(message);
    this.name = 'FlarionApiError';
  }
}
