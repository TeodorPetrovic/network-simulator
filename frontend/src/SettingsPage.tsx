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
  if (typeof value === "string" && name === "iourc_content") return <label className="grid gap-1.5" htmlFor={id}><span className="text-sm font-medium">IOU license file (.iourc content)</span><Textarea id={id} value={value} rows={5} onChange={(event) => onChange(path, event.target.value)} /></label>;
  if (typeof value === "string") return <label className="grid gap-1.5" htmlFor={id}><span className="text-sm font-medium">{displayLabel(name)}{secret && <Badge className="ml-2" variant="outline">Read only</Badge>}</span><Input id={id} type={/password/i.test(name) ? "password" : "text"} value={value} disabled={secret} onChange={(event) => onChange(path, event.target.value)} /></label>;
  return <div className="settings-uneditable"><span>{displayLabel(name)}</span><Badge variant="outline">Read only</Badge></div>;
}

export function SettingsPage() {
  const [draft, setDraft] = useState<ServerSettings | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [authEnabled, setAuthEnabled] = useState<boolean | null>(null);
  const [authError, setAuthError] = useState("");
  const [authBusy, setAuthBusy] = useState(false);
  async function refresh() {
    setLoading(true); setError("");
    try { const result = await api.settings(); setDraft(result); }
    catch (reason) { setError(`Controller settings are unavailable: ${errorMessage(reason)}`); setDraft(null); }
    finally { setLoading(false); }
    try { setAuthEnabled((await api.authSettings()).auth_enabled); setAuthError(""); }
    catch (reason) { setAuthError(`Authentication setting is unavailable: ${errorMessage(reason)}`); }
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
  async function saveAuth(enabled: boolean) {
    setAuthBusy(true); setAuthError("");
    try { setAuthEnabled((await api.updateAuthSettings(enabled)).auth_enabled); }
    catch (reason) { setAuthError(`Authentication setting could not be saved: ${errorMessage(reason)}`); }
    finally { setAuthBusy(false); }
  }
  const groups = draft ? Object.entries(draft) : [];
  const restartSettingPresent = groups.some(([, value]) => isRecord(value) && Object.keys(value).some((key) => ["host", "port", "protocol", "projects_path", "images_path", "ubridge_path"].includes(key.toLowerCase())));
  return <PagesShell active="Settings"><div className="pages-content">
    <PageTitle eyebrow="CONTROLLER" title="General settings" description="Values are loaded from the controller settings endpoint; only fields returned by the backend are shown." actions={<><Button size="sm" variant="outline" onClick={() => void refresh()} disabled={loading}><RefreshCw size={15} />Reload</Button><Button size="sm" onClick={() => void save()} disabled={!draft || busy}><Save size={15} />{busy ? "Saving…" : "Save settings"}</Button></>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}</div>}
    <Card className="settings-group access-setting-card"><div className="settings-group-heading"><div><h2>API authentication</h2><p>Protect controller API and WebSocket routes with user tokens.</p></div><label className="settings-boolean"><input aria-label="Enable API authentication" type="checkbox" checked={authEnabled ?? false} disabled={authEnabled === null || authBusy} onChange={(event) => void saveAuth(event.target.checked)} /><span>{authBusy ? "Saving…" : authEnabled ? "Enabled" : "Disabled"}</span></label></div>{authError && <div role="alert" className="pages-alert">{authError}</div>}<p className="access-setting-help">Loopback-only binds default to authentication off. Non-loopback binds force authentication on at every start unless <code>NETWORK_SIMULATOR_AUTH_ENABLED=false</code> is explicit. Forwarded proxy headers also require authentication on loopback unless <code>NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH=true</code> is set.</p></Card>
    {restartSettingPresent && <div className="pages-warning"><Settings2 size={16} />Some server paths and listener settings require a controller restart before they take effect.</div>}
    {loading ? <Card className="pages-empty-state">Loading controller settings…</Card> : draft && groups.length ? <div className="settings-groups">{groups.map(([group, value]) => <Card className="settings-group" key={group}><div className="settings-group-heading"><div><h2>{displayLabel(group)}</h2><p>Controller configuration values consumed by this installation.</p></div><Badge variant="outline">Backend field</Badge></div>{isRecord(value) ? <div className="settings-fields">{Object.entries(value).map(([key, child]) => <SettingValue key={key} name={key} value={child} path={[group, key]} onChange={change} />)}</div> : <div className="settings-fields"><SettingValue name={group} value={value} path={[group]} onChange={change} /></div>}</Card>)}</div> : !error ? <Card className="pages-empty-state">The controller returned no editable settings.</Card> : <Card className="pages-empty-state"><Settings2 size={18} />No settings form is available until the controller exposes the `/v3/settings` endpoint.</Card>}
  </div></PagesShell>;
}
