import { createFileRoute, Link } from "@tanstack/react-router";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { getDesktopPrefs, installTool, listenToolProgress, listTools, saveDesktopPrefs } from "../api/desktop";
import type { DesktopPrefs, ToolId, ToolStatus } from "../api/desktop";
import { isDesktop } from "../api/server";

export const Route = createFileRoute("/desktop")({ component: DesktopSettingsPage });

type Progress = { stage: string; done: number; total: number };

function describe(progress: Progress | undefined) {
  if (!progress) return "";
  if (progress.stage === "download") return progress.total ? `Downloading… ${Math.round((progress.done / progress.total) * 100)}%` : "Downloading…";
  if (progress.stage === "extract") return "Unpacking…";
  if (progress.stage === "install") return "Installing (approve the Windows prompt)…";
  return "";
}

function Choice<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: Array<{ value: T; text: string; disabled?: boolean }>; onChange: (value: T) => void }) {
  return <fieldset className="mt-4">
    <legend className="text-sm font-medium">{label}</legend>
    <div className="mt-2 flex flex-wrap gap-2">
      {options.map((option) => <label key={option.value} className={`flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-sm ${value === option.value ? "border-primary bg-accent" : ""} ${option.disabled ? "opacity-50" : ""}`}>
        <input type="radio" name={label} checked={value === option.value} disabled={option.disabled} onChange={() => onChange(option.value)} />{option.text}
      </label>)}
    </div>
  </fieldset>;
}

export function DesktopSettingsPage() {
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [prefs, setPrefs] = useState<DesktopPrefs>(getDesktopPrefs);
  const [progress, setProgress] = useState<Partial<Record<ToolId, Progress>>>({});
  const [busy, setBusy] = useState<ToolId | null>(null);
  const [error, setError] = useState("");
  const desktop = isDesktop();
  const refresh = useCallback(() => { listTools().then(setTools).catch((reason: unknown) => setError(String(reason))); }, []);
  useEffect(() => {
    if (!desktop) return;
    refresh();
    let stop = () => {};
    void listenToolProgress((event) => setProgress((current) => ({ ...current, [event.id as ToolId]: event }))).then((unlisten) => { stop = unlisten; });
    return () => stop();
  }, [desktop, refresh]);

  const status = (id: ToolId) => tools.find((tool) => tool.id === id);
  const update = (next: Partial<DesktopPrefs>) => { const merged = { ...prefs, ...next }; setPrefs(merged); saveDesktopPrefs(merged); };
  async function install(id: ToolId) {
    setBusy(id); setError("");
    try { setTools(await installTool(id)); } catch (reason) { setError(typeof reason === "string" ? reason : reason instanceof Error ? reason.message : "Install failed."); }
    finally { setBusy(null); setProgress((current) => ({ ...current, [id]: undefined })); }
  }

  if (!desktop) return <main className="mx-auto max-w-2xl p-6"><h1 className="text-xl font-semibold">Desktop tools</h1><p className="mt-2 text-sm text-muted-foreground">These settings are only available in the Network Studio desktop app.</p><Link to="/" className="mt-4 inline-block text-sm underline">Back</Link></main>;

  const motty = status("motty")?.installed ?? false;
  const wireshark = status("wireshark")?.installed ?? false;
  return <main className="mx-auto max-w-2xl space-y-4 p-6">
    <div className="flex items-center justify-between"><h1 className="text-xl font-semibold">Desktop tools</h1><Link to="/" className="text-sm underline">Back</Link></div>
    <p className="text-sm text-muted-foreground">Devices run on the server. These tools run on this computer and connect to it.</p>
    {error && <p role="alert" className="rounded-md border border-destructive p-3 text-sm text-destructive">{error}</p>}
    {(["wireshark", "motty"] as ToolId[]).map((id) => {
      const tool = status(id);
      if (!tool) return null;
      return <Card key={id} className="p-4">
        <div className="flex items-start justify-between gap-4">
          <div><h2 className="font-medium">{tool.name} <span className="text-xs font-normal text-muted-foreground">{tool.installed ? "installed" : "not installed"}</span></h2>
            <p className="mt-1 text-sm text-muted-foreground">{tool.note}</p>
            {tool.path && <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{tool.path}</p>}
            {busy === id && <p className="mt-2 text-sm">{describe(progress[id])}</p>}</div>
          {!tool.installed && tool.can_install && <Button size="sm" disabled={busy !== null} onClick={() => void install(id)}>{busy === id ? "Working…" : "Download and install"}</Button>}
          {tool.installed && <Button size="sm" variant="outline" onClick={refresh}>Re-check</Button>}
        </div>
      </Card>;
    })}
    <Card className="p-4">
      <h2 className="font-medium">Which tool opens what</h2>
      <Choice label="Device consoles (telnet)" value={prefs.console} onChange={(console) => update({ console })}
        options={[{ value: "builtin", text: "Built-in terminal (xterm)" }, { value: "motty", text: "Motty", disabled: !motty }]} />
      <Choice label="Graphical consoles (VNC)" value={prefs.vnc} onChange={(vnc) => update({ vnc })}
        options={[{ value: "builtin", text: "Built-in viewer (noVNC)" }, { value: "motty", text: "Motty", disabled: !motty }]} />
      <Choice label="Packet captures" value={prefs.capture} onChange={(capture) => update({ capture })}
        options={[{ value: "builtin", text: "Built-in viewer" }, { value: "wireshark", text: "Wireshark", disabled: !wireshark }]} />
      <p className="mt-4 text-xs text-muted-foreground">Motty and Wireshark connect from this computer, so the device's console port must be reachable on the server. The built-in terminal and viewers go through the controller and always work.</p>
    </Card>
  </main>;
}
