import { fetchWithAuth, handleUnauthorized, request, jsonBody } from "./client";

export type LlmProvider = "openai" | "anthropic" | "ollama";
export type LlmConfig = { config_id: string; name: string; provider: LlmProvider; base_url: string; model: string; has_api_key: boolean; is_default: boolean };
export type LlmConfigInput = { name?: string; provider: LlmProvider; base_url?: string; model: string; api_key?: string; is_default?: boolean };
export type ChatSession = { session_id: string; title: string; pinned: boolean; updated_at: string };
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error?: boolean };
export type ChatMessage = { role: "user" | "assistant"; content: ContentBlock[] };
export type ChatEvent =
  | { kind: "session"; session_id: string }
  | { kind: "text"; text: string }
  | { kind: "tool_call"; id: string; name: string; input: unknown }
  | { kind: "tool_result"; id: string; name: string; is_error: boolean; content: string }
  | { kind: "error"; message: string }
  | { kind: "done" };

const base = (projectId: string) => `/copilot/projects/${projectId}/chat`;

export const copilotApi = {
  llmConfigs: () => request<LlmConfig[]>("/copilot/llm-configs"),
  createLlmConfig: (input: LlmConfigInput) => request<LlmConfig>("/copilot/llm-configs", jsonBody(input)),
  updateLlmConfig: (id: string, input: Partial<LlmConfigInput>) => request<LlmConfig>(`/copilot/llm-configs/${id}`, { method: "PUT", body: JSON.stringify(input) }),
  deleteLlmConfig: (id: string) => request<void>(`/copilot/llm-configs/${id}`, { method: "DELETE" }),
  chatSessions: (projectId: string) => request<ChatSession[]>(`${base(projectId)}/sessions`),
  chatHistory: (projectId: string, sessionId: string) => request<{ session_id: string; messages: ChatMessage[] }>(`${base(projectId)}/sessions/${sessionId}`),
  deleteChatSession: (projectId: string, sessionId: string) => request<void>(`${base(projectId)}/sessions/${sessionId}`, { method: "DELETE" }),
  updateChatSession: (projectId: string, sessionId: string, patch: { title?: string; pinned?: boolean }) =>
    request<void>(`${base(projectId)}/sessions/${sessionId}`, { method: "PATCH", body: JSON.stringify(patch) }),
  async streamChat(projectId: string, body: { message: string; session_id?: string; config_id?: string; read_only?: boolean }, onEvent: (event: ChatEvent) => void, signal?: AbortSignal) {
    const path = `${base(projectId)}/stream`;
    const response = await fetchWithAuth(`/v3${path}`, { method: "POST", body: JSON.stringify(body), signal, headers: { "Content-Type": "application/json", Accept: "text/event-stream" } });
    handleUnauthorized(response, path);
    if (!response.ok || !response.body) {
      const failure = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(failure?.message ?? `Request failed (${response.status})`);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const parsed = parseSseFrames(buffer);
      buffer = parsed.rest;
      for (const event of parsed.events) onEvent(event);
    }
  },
};

/** Splits complete SSE frames off the buffer; the unfinished tail is returned as `rest`. */
export function parseSseFrames(buffer: string): { events: ChatEvent[]; rest: string } {
  const normalized = buffer.replace(/\r\n/g, "\n");
  const frames = normalized.split("\n\n");
  const rest = frames.pop() ?? "";
  const events: ChatEvent[] = [];
  for (const frame of frames) {
    let kind = "message";
    const data: string[] = [];
    for (const line of frame.split("\n")) {
      if (line.startsWith("event:")) kind = line.slice(6).trim();
      else if (line.startsWith("data:")) data.push(line.slice(5).trimStart());
    }
    if (!data.length) continue;
    try { events.push({ kind, ...JSON.parse(data.join("\n")) } as ChatEvent); } catch { /* ignore malformed frame */ }
  }
  return { events, rest };
}
