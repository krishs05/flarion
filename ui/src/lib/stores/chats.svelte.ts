import { uuid } from '$lib/utils/uuid';

const STORAGE_KEY = 'flarion.chats';

export interface Metrics {
  ttft: number;
  tokensPerSecond: number;
  totalTokens: number;
  durationMs: number;
}

export interface Message {
  role: 'system' | 'user' | 'assistant';
  content: string;
  metrics?: Metrics;
}

export interface Chat {
  id: string;
  title: string;
  model: string;
  messages: Message[];
  createdAt: number;
  updatedAt: number;
}

function loadChats(): Chat[] {
  if (typeof localStorage === 'undefined') return [];
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    return JSON.parse(raw) as Chat[];
  } catch {
    return [];
  }
}

export const chatStore = $state<{ chats: Chat[]; activeId: string | null }>({
  chats: loadChats(),
  activeId: null
});

export function persistChats() {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(chatStore.chats));
}

export function newChat(model: string): Chat {
  const chat: Chat = {
    id: uuid(),
    title: 'new chat',
    model,
    messages: [],
    createdAt: Date.now(),
    updatedAt: Date.now()
  };
  chatStore.chats.unshift(chat);
  chatStore.activeId = chat.id;
  persistChats();
  return chat;
}

export function getActive(): Chat | null {
  if (!chatStore.activeId) return null;
  return chatStore.chats.find((c) => c.id === chatStore.activeId) ?? null;
}

export function appendMessage(chatId: string, message: Message) {
  const chat = chatStore.chats.find((c) => c.id === chatId);
  if (!chat) return;
  chat.messages.push(message);
  chat.updatedAt = Date.now();

  if (chat.title === 'new chat' && message.role === 'user') {
    chat.title = message.content.slice(0, 40).trim() || 'new chat';
  }
  persistChats();
}

export function appendToLastMessage(chatId: string, delta: string) {
  const chat = chatStore.chats.find((c) => c.id === chatId);
  if (!chat || chat.messages.length === 0) return;
  const last = chat.messages[chat.messages.length - 1];
  last.content += delta;
  chat.updatedAt = Date.now();
}

export function finalizeLastMessage(chatId: string, metrics: Metrics) {
  const chat = chatStore.chats.find((c) => c.id === chatId);
  if (!chat || chat.messages.length === 0) return;
  const last = chat.messages[chat.messages.length - 1];
  last.metrics = metrics;
  chat.updatedAt = Date.now();
  persistChats();
}

export function deleteChat(chatId: string) {
  const idx = chatStore.chats.findIndex((c) => c.id === chatId);
  if (idx === -1) return;
  chatStore.chats.splice(idx, 1);
  if (chatStore.activeId === chatId) {
    chatStore.activeId = chatStore.chats[0]?.id ?? null;
  }
  persistChats();
}

export function setActive(chatId: string) {
  chatStore.activeId = chatId;
}
