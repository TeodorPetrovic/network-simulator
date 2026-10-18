import { useEffect, useState } from "react";
import type { ChangeEvent } from "react";
import { RefreshCw, Save, Settings2 } from "lucide-react";
import type { ServerSettings } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";

const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";
const isRecord = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === "object" && !Array.isArray(value);
function setAtPath(source: Record<string, unknown>, path: string[], value: unknown): Record<string, unknown> {
  if (!path.length) return source;
  const [head, ...tail] = path;
  const next = { ...source };
  if (!tail.length) next[head] = value;
  else next[head] = setAtPath(isRecord(source[head]) ? source[head] as Record<string, unknown> : {}, tail, value);
  return next;
}
const displayLabel = (key: string) => key.replaceAll("_", " ").replaceAll(".", " / ").replace(/\b\w/g, (letter) => letter.toUpperCase());
const looksSecret = (key: string, value: unknown) => /(password|secret|token|private.?key)/i.test(key) || (typeof value === "string" && /^\*{3,}$/.test(value));

function SettingValue({ name, value, path, onChange }: { name: string; value: unknown; path: string[]; onChange: (path: string[], value: unknown) => void }) {
  const secret = looksSecret(name, value);
  const id = `setting-${path.join("-")}`;
  if (isRecord(value)) return <div className="settings-nested"><h3>{displayLabel(name)}</h3><div className="settings-fields">{Object.entries(value).map(([childKey, child]) => <SettingValue key={childKey} name={childKey} value={child} path={[...path, childKey]} onChange={onChange} />)}</div></div>;
  if (typeof value === "boolean") return <label className="settings-boolean" htmlFor={id}><input id={id} type="checkbox" checked={value} disabled={secret} onChange={(event) => onChange(path, event.target.checked)} /><span>{displayLabel(name)}</span>{secret && <Badge variant="outline">Read only</Badge>}</label>;
  if (typeof value === "number") return <label className="grid gap-1.5" htmlFor={id}><span className="text-sm font-medium">{displayLabel(name)}{secret && <Badge className="ml-2" variant="outline">Read only</Badge>}</span><Input id={id} type="number" value={value} disabled={secret} onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(path, Number(event.target.value))} /></label>;
  if (Array.isArray(value)) return <label className="grid gap-1.5" htmlFor={id}><span className="text-sm font-medium">{displayLabel(name)} <Badge variant="outline">Read only</Badge></span><Textarea id={id} value={JSON.stringify(value, null, 2)} readOnly rows={3} /></label>;
  if (typeof value === "string") return <label className="grid gap-1.5" htmlFor={id}><span className="text-sm font-medium">{displayLabel(name)}{secret && <Badge className="ml-2" variant="outline">Read only</Badge>}</span><Input id={id} type={/password/i.test(name) ? "password" : "text"} value={value} disabled={secret} onChange={(event) => onChange(path, event.target.value)} /></label>;
  return <div className="settings-uneditable"><span>{displayLabel(name)}</span><Badge variant="outline">Read only</Badge></div>;
}

export function SettingsPage() {
  const [draft, setDraft] = useState<ServerSettings | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  async function refresh() {
    setLoading(true); setError("");
    try { const result = await api.settings(); setDraft(result); }
    catch (reason) { setError(`Controller settings are unavailable: ${errorMessage(reason)}`); setDraft(null); }
    finally { setLoading(false); }
  }
  useEffect(() => { void refresh(); }, []);
  function change(path: string[], value: unknown) { setDraft((current) => current ? setAtPath(current, path, value) : current); }
  async function save() {
    if (!draft) return;
    setBusy(true); setError(""); setNotice("");
    try { const result = await api.updateSettings(draft); setDraft(result); setNotice("Controller settings saved."); }
    catch (reason) { setError(`Settings could not be saved: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  const groups = draft ? Object.entries(draft) : [];
  const restartSettingPresent = groups.some(([, value]) => isRecord(value) && Object.keys(value).some((key) => ["host", "port", "protocol", "projects_path", "images_path", "ubridge_path"].includes(key.toLowerCase())));
  return <PagesShell active="Settings"><div className="pages-content">
    <PageTitle eyebrow="CONTROLLER" title="General settings" description="Values are loaded from the controller settings endpoint; only fields returned by the backend are shown." actions={<><Button size="sm" variant="outline" onClick={() => void refresh()} disabled={loading}><RefreshCw size={15} />Reload</Button><Button size="sm" onClick={() => void save()} disabled={!draft || busy}><Save size={15} />{busy ? "Saving…" : "Save settings"}</Button></>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}</div>}
    {restartSettingPresent && <div className="pages-warning"><Settings2 size={16} />Some server paths and listener settings require a controller restart before they take effect.</div>}
    {loading ? <Card className="pages-empty-state">Loading controller settings…</Card> : draft && groups.length ? <div className="settings-groups">{groups.map(([group, value]) => <Card className="settings-group" key={group}><div className="settings-group-heading"><div><h2>{displayLabel(group)}</h2><p>Controller configuration values consumed by this installation.</p></div><Badge variant="outline">Backend field</Badge></div>{isRecord(value) ? <div className="settings-fields">{Object.entries(value).map(([key, child]) => <SettingValue key={key} name={key} value={child} path={[group, key]} onChange={change} />)}</div> : <div className="settings-fields"><SettingValue name={group} value={value} path={[group]} onChange={change} /></div>}</Card>)}</div> : !error ? <Card className="pages-empty-state">The controller returned no editable settings.</Card> : <Card className="pages-empty-state"><Settings2 size={18} />No settings form is available until the controller exposes the `/v3/settings` endpoint.</Card>}
  </div></PagesShell>;
}
