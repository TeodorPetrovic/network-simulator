import { useCallback, useEffect, useRef, useState } from "react";
import { Bot, ChevronDown, ChevronUp, Plus, Send, Settings, Square, Trash2, Wrench, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api";
import type { ChatMessage, ChatSession, ContentBlock, LlmConfig } from "../api";
import { clampFloat, useFloatingFront } from "../floating";
import { CopilotSettings } from "./CopilotSettings";
import "./copilot.css";

type Props = { projectId: string; onClose: () => void; onTopologyMayHaveChanged?: () => void };

/** Collapses stored messages into a flat list: assistant text, tool calls with their results, user text. */
export function toDisplayItems(messages: ChatMessage[]): DisplayItem[] {
  const items: DisplayItem[] = [];
  const results = new Map<string, Extract<ContentBlock, { type: "tool_result" }>>();
  for (const message of messages) for (const block of message.content) if (block.type === "tool_result") results.set(block.tool_use_id, block);
  for (const message of messages) {
    for (const block of message.content) {
      if (block.type === "text" && block.text) items.push({ kind: message.role, text: block.text });
      if (block.type === "tool_use") {
        const result = results.get(block.id);
        items.push({ kind: "tool", id: block.id, name: block.name, input: block.input, result: result?.content, error: result?.is_error });
      }
    }
  }
  return items;
}
export type DisplayItem =
  | { kind: "user" | "assistant"; text: string }
  | { kind: "tool"; id: string; name: string; input: unknown; result?: string; error?: boolean };

export function CopilotWindow({ projectId, onClose, onTopologyMayHaveChanged }: Props) {
  const { z, front } = useFloatingFront();
  const [position, setPosition] = useState({ x: Math.max(24, innerWidth - 480), y: 70, width: 440, height: 600 });
  const [minimized, setMinimized] = useState(false);
  const [configs, setConfigs] = useState<LlmConfig[] | null>(null);
  const [configId, setConfigId] = useState<string>("");
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [sessionId, setSessionId] = useState<string | undefined>();
  const [items, setItems] = useState<DisplayItem[]>([]);
  const [input, setInput] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const endRef = useRef<HTMLDivElement>(null);

  const loadConfigs = useCallback(async () => {
    try {
      const list = await api.llmConfigs();
      setConfigs(list);
      setConfigId((current) => (list.some((c) => c.config_id === current) ? current : list.find((c) => c.is_default)?.config_id ?? list[0]?.config_id ?? ""));
      if (!list.length) setSettingsOpen(true);
    } catch (reason) { setConfigs([]); setError(reason instanceof Error ? reason.message : "Could not load model settings."); }
  }, []);
  const loadSessions = useCallback(() => { void api.chatSessions(projectId).then(setSessions).catch(() => {}); }, [projectId]);
  useEffect(() => { void loadConfigs(); loadSessions(); }, [loadConfigs, loadSessions]);
  useEffect(() => { endRef.current?.scrollIntoView({ block: "end" }); }, [items, busy]);
  useEffect(() => () => abort.current?.abort(), []);

  async function openSession(id: string) {
    setError("");
    try {
      const history = await api.chatHistory(projectId, id);
      setSessionId(id);
      setItems(toDisplayItems(history.messages));
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Could not open the chat."); }
  }
  function newChat() { abort.current?.abort(); setSessionId(undefined); setItems([]); setError(""); setBusy(false); }

  async function send() {
    const message = input.trim();
    if (!message || busy) return;
    if (!configId) { setSettingsOpen(true); return; }
    setInput(""); setError(""); setBusy(true);
    setItems((current) => [...current, { kind: "user", text: message }]);
    const controller = new AbortController();
    abort.current = controller;
    let changed = false;
    try {
      await api.streamChat(projectId, { message, session_id: sessionId, config_id: configId, read_only: readOnly }, (event) => {
        if (event.kind === "session") setSessionId(event.session_id);
        else if (event.kind === "text") setItems((current) => [...current, { kind: "assistant", text: event.text }]);
        else if (event.kind === "tool_call") setItems((current) => [...current, { kind: "tool", id: event.id, name: event.name, input: event.input }]);
        else if (event.kind === "tool_result") {
          if (!/_(list|get|stats|links|version)$|^node_file_list$|^link_available_filters$|^node_run_command$/.test(event.name)) changed = true;
          setItems((current) => current.map((item) => (item.kind === "tool" && item.id === event.id ? { ...item, result: event.content, error: event.is_error } : item)));
        } else if (event.kind === "error") setError(event.message);
      }, controller.signal);
    } catch (reason) {
      if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "The copilot request failed.");
    } finally {
      setBusy(false); loadSessions();
      if (changed) onTopologyMayHaveChanged?.();
    }
  }

  const noModel = configs !== null && configs.length === 0;
  return <section className={`capture-window copilot-window ${minimized ? "minimized" : ""}`} style={{ left: position.x, top: position.y, width: minimized ? 300 : position.width, height: minimized ? 42 : position.height, zIndex: z }} onPointerDownCapture={front} aria-label="AI copilot">
    <header onPointerDown={(event) => {
      if ((event.target as HTMLElement).closest("button,select")) return;
      const x = event.clientX, y = event.clientY, startX = position.x, startY = position.y;
      const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, ...clampFloat(startX + pointer.clientX - x, startY + pointer.clientY - y, current.width) }));
      const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); };
      window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop);
    }}><Bot size={16} />Copilot<span />
      {!minimized && <Button size="icon-xs" variant="ghost" aria-label="New chat" title="New chat" onClick={newChat}><Plus /></Button>}
      {!minimized && <Button size="icon-xs" variant="ghost" aria-label="Copilot settings" title="Model settings" onClick={() => setSettingsOpen((open) => !open)}><Settings /></Button>}
      <Button size="icon-xs" variant="ghost" aria-label={minimized ? "Restore Copilot" : "Minimize Copilot"} onClick={() => setMinimized((value) => !value)}>{minimized ? <ChevronUp /> : <ChevronDown />}</Button>
      <Button size="icon-xs" variant="ghost" aria-label="Close Copilot" onClick={onClose}><X /></Button>
    </header>
    {!minimized && <div className="copilot-body">
      {settingsOpen && <CopilotSettings configs={configs ?? []} onChanged={() => void loadConfigs()} onDone={() => setSettingsOpen(false)} />}
      {!settingsOpen && <>
        <div className="copilot-bar">
          <select aria-label="Chat history" value={sessionId ?? ""} onChange={(event) => (event.target.value ? void openSession(event.target.value) : newChat())}>
            <option value="">New chat</option>
            {sessions.map((session) => <option key={session.session_id} value={session.session_id}>{session.pinned ? "★ " : ""}{session.title}</option>)}
          </select>
          {sessionId && <Button size="icon-xs" variant="ghost" aria-label="Delete chat" title="Delete this chat" onClick={() => { const id = sessionId; newChat(); void api.deleteChatSession(projectId, id).then(loadSessions); }}><Trash2 /></Button>}
        </div>
        <div className="copilot-messages" role="log" aria-live="polite">
          {noModel && <p className="copilot-hint">Add an AI model in settings to start. Anthropic, OpenAI and any OpenAI-compatible server (Ollama, OpenRouter…) work.</p>}
          {!noModel && !items.length && <p className="copilot-hint">Ask me to build a topology, start devices, check links or read a device config.</p>}
          {items.map((item, index) => item.kind === "tool"
            ? <details key={item.id + index} className={`copilot-tool ${item.error ? "is-error" : ""}`}><summary><Wrench size={12} />{item.name}{item.result === undefined ? " …" : item.error ? " failed" : ""}</summary><pre>{JSON.stringify(item.input, null, 2)}</pre>{item.result !== undefined && <pre>{item.result}</pre>}</details>
            : <div key={index} className={`copilot-msg is-${item.kind}`}>{item.text}</div>)}
          {busy && <div className="copilot-msg is-assistant copilot-typing">Working…</div>}
          {error && <div className="copilot-error" role="alert">{error}</div>}
          <div ref={endRef} />
        </div>
        <form className="copilot-input" onSubmit={(event) => { event.preventDefault(); void send(); }}>
          <textarea aria-label="Message the copilot" rows={2} value={input} placeholder="Message the copilot…" onChange={(event) => setInput(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void send(); } }} />
          {busy ? <Button type="button" size="icon" variant="outline" aria-label="Stop" onClick={() => abort.current?.abort()}><Square /></Button> : <Button type="submit" size="icon" aria-label="Send" disabled={!input.trim()}><Send /></Button>}
        </form>
        <div className="copilot-foot">
          <label><input type="checkbox" checked={readOnly} onChange={(event) => setReadOnly(event.target.checked)} /> Read-only</label>
          {(configs?.length ?? 0) > 1 && <select aria-label="Model" value={configId} onChange={(event) => setConfigId(event.target.value)}>{configs!.map((c) => <option key={c.config_id} value={c.config_id}>{c.name}</option>)}</select>}
          {(configs?.length ?? 0) === 1 && <span className="copilot-model">{configs![0].model}</span>}
        </div>
      </>}
    </div>}
    {!minimized && <div className="capture-window-resize" onPointerDown={(event) => { event.preventDefault(); const x = event.clientX, y = event.clientY, width = position.width, height = position.height; const move = (pointer: PointerEvent) => setPosition((current) => ({ ...current, width: Math.max(340, Math.min(innerWidth - current.x, width + pointer.clientX - x)), height: Math.max(280, Math.min(innerHeight - current.y, height + pointer.clientY - y)) })); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} />}
  </section>;
}
