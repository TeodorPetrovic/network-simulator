import { useState } from "react";
import { Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "../api";
import type { LlmConfig, LlmProvider } from "../api";

const PRESETS: Record<LlmProvider, { base: string; model: string; hint: string }> = {
  anthropic: { base: "", model: "claude-sonnet-5-5", hint: "Anthropic API key (sk-ant-…)" },
  openai: { base: "", model: "gpt-4.1", hint: "OpenAI-compatible API key. Set the URL for OpenRouter, DeepSeek, Gemini…" },
  ollama: { base: "http://127.0.0.1:11434/v1", model: "llama3.1", hint: "No key needed for a local Ollama server" },
};

export function CopilotSettings({ configs, onChanged, onDone }: { configs: LlmConfig[]; onChanged: () => void; onDone: () => void }) {
  const [provider, setProvider] = useState<LlmProvider>("anthropic");
  const [model, setModel] = useState(PRESETS.anthropic.model);
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

  function choose(next: LlmProvider) { setProvider(next); setModel(PRESETS[next].model); setBaseUrl(PRESETS[next].base); }
  async function add() {
    setError(""); setSaving(true);
    try {
      await api.createLlmConfig({ provider, model: model.trim(), base_url: baseUrl.trim(), api_key: apiKey, name: `${provider} · ${model.trim()}` });
      setApiKey(""); onChanged(); onDone();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Could not save the model."); }
    finally { setSaving(false); }
  }
  return <div className="copilot-settings">
    <h3>Models</h3>
    {configs.map((config) => <div key={config.config_id} className="copilot-config-row">
      <span><strong>{config.name}</strong>{config.is_default && <em> default</em>}</span>
      {!config.is_default && <Button size="xs" variant="ghost" onClick={() => void api.updateLlmConfig(config.config_id, { is_default: true }).then(onChanged)}>Make default</Button>}
      <Button size="icon-xs" variant="ghost" aria-label={`Delete ${config.name}`} onClick={() => void api.deleteLlmConfig(config.config_id).then(onChanged)}><Trash2 /></Button>
    </div>)}
    <h3>Add a model</h3>
    <label>Provider<select value={provider} onChange={(event) => choose(event.target.value as LlmProvider)}><option value="anthropic">Anthropic</option><option value="openai">OpenAI-compatible</option><option value="ollama">Ollama (local)</option></select></label>
    <label>Model<Input value={model} onChange={(event) => setModel(event.target.value)} /></label>
    {provider !== "anthropic" && <label>Base URL<Input value={baseUrl} placeholder="https://api.openai.com/v1" onChange={(event) => setBaseUrl(event.target.value)} /></label>}
    <label>API key<Input type="password" autoComplete="off" value={apiKey} placeholder={PRESETS[provider].hint} onChange={(event) => setApiKey(event.target.value)} /></label>
    <p className="copilot-hint">Keys are stored on the controller and are never sent back to the browser.</p>
    {error && <div className="copilot-error" role="alert">{error}</div>}
    <div className="copilot-settings-actions"><Button disabled={saving || !model.trim()} onClick={() => void add()}>Save model</Button>{configs.length > 0 && <Button variant="ghost" onClick={onDone}>Back</Button>}</div>
  </div>;
}
