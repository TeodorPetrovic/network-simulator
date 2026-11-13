import { useEffect, useState } from "react";
import { Activity, Cpu, HardDrive, RefreshCw, Server } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ResourceMonitor, RuntimeInfo } from "./api";
import { api } from "./api";
import { PagesShell, PageLayout, PageSection, PageTitle } from "./PagesShell";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";

type Sample = { at: number; cpu: number | null; memory: number | null };
const bytes = (value?: number | null) => value === null || value === undefined ? "—" : `${(value / 1024 ** 3).toFixed(1)} GiB`;
const percent = (value?: number | null) => value === null || value === undefined ? null : Math.max(0, Math.min(100, value));
const humanError = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

function MetricCard({ label, value, detail, icon: Icon, ratio }: { label: string; value: string; detail: string; icon: LucideIcon; ratio?: number | null }) {
  return <Card className="min-w-0 space-y-3 p-4"><div className="flex items-center justify-between gap-2 text-sm text-muted-foreground"><span>{label}</span><Icon size={17} /></div><div className="break-words text-xl font-semibold tracking-tight">{value}</div><div className="text-xs text-muted-foreground">{detail}</div>{ratio !== undefined && ratio !== null && <Progress value={ratio} aria-label={`${label} ${ratio.toFixed(1)} percent`} />}</Card>;
}

export function SystemStatusPage() {
  const [resources, setResources] = useState<ResourceMonitor | null>(null);
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [samples, setSamples] = useState<Sample[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true); setError("");
    const outcomes = await Promise.allSettled([api.resourceMonitor(), api.runtimes()]);
    if (outcomes[0].status === "fulfilled") {
      const next = outcomes[0].value; setResources(next);
      setSamples((current) => [...current, { at: Date.now(), cpu: next.host.cpu_usage_percent, memory: next.host.memory_total_bytes ? (next.host.memory_used_bytes ?? 0) / next.host.memory_total_bytes * 100 : null }].slice(-30));
    } else setError(`Resource monitor unavailable: ${humanError(outcomes[0].reason)}`);
    if (outcomes[1].status === "fulfilled") setRuntime(outcomes[1].value);
    else { const detail = humanError(outcomes[1].reason); setError((current) => [current, `Runtime status unavailable: ${detail}`].filter(Boolean).join(" · ")); }
    setBusy(false);
  }
  useEffect(() => { void refresh(); const timer = window.setInterval(() => void refresh(), 5000); return () => window.clearInterval(timer); }, []);

  const memoryRatio = resources?.host.memory_total_bytes ? (resources.host.memory_used_bytes ?? 0) / resources.host.memory_total_bytes * 100 : null;
  const runtimeRows = runtime ? Object.entries(runtime) : [];
  return <PagesShell active="System status"><PageLayout>
    <PageTitle eyebrow="CONTROLLER" title="System status" description="Live host resources, controller usage, and available runtime services." actions={<Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}><RefreshCw size={15} />Refresh now</Button>} />
    {error && <Alert variant="destructive" className="mt-6"><AlertDescription>{error}</AlertDescription></Alert>}
    {!resources ? <Card className="mt-6 flex items-center gap-3 p-5 text-sm text-muted-foreground"><Activity className="size-5" /><span>Waiting for resource status from the controller…</span></Card> : <>
      <div className="mt-6 grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4"><MetricCard label="CPU usage" value={resources.host.cpu_usage_percent === null ? "—" : `${resources.host.cpu_usage_percent.toFixed(1)}%`} detail={`${resources.host.cpu_count ?? "—"} logical processors`} icon={Cpu} ratio={percent(resources.host.cpu_usage_percent)} /><MetricCard label="Host memory" value={`${bytes(resources.host.memory_used_bytes)} / ${bytes(resources.host.memory_total_bytes)}`} detail={memoryRatio === null ? "Usage unavailable" : `${memoryRatio.toFixed(1)}% in use`} icon={HardDrive} ratio={percent(memoryRatio)} /><MetricCard label="Controller memory" value={bytes(resources.controller.memory_rss_bytes)} detail={`${resources.controller.thread_count ?? "—"} controller threads`} icon={Server} /><MetricCard label="Load average" value={resources.host.load_average?.map((value) => value.toFixed(2)).join(" · ") ?? "—"} detail="1, 5, and 15 minute averages" icon={Activity} /></div>
      <PageSection><Card className="min-w-0 space-y-4 p-4 sm:p-5"><div className="flex flex-wrap items-start justify-between gap-2"><div><h2 className="font-semibold">Recent resource samples</h2><p className="mt-1 text-sm text-muted-foreground">Samples collected while this page is open.</p></div><span className="text-xs text-muted-foreground">5 second polling · {samples.length} samples</span></div><div className="flex h-36 items-end gap-1 border-b border-border" role="img" aria-label="Recent CPU and memory usage">{samples.map((sample, index) => <div key={`${sample.at}-${index}`} className="flex h-full min-w-0 flex-1 items-end gap-px" title={`CPU ${sample.cpu?.toFixed(1) ?? "—"}%, memory ${sample.memory?.toFixed(1) ?? "—"}%`}><span className="w-1/2 rounded-t bg-success" style={{ height: `${sample.memory ?? 0}%` }} /><span className="w-1/2 rounded-t bg-primary" style={{ height: `${sample.cpu ?? 0}%` }} /></div>)}</div><div className="flex gap-4 text-xs text-muted-foreground"><span className="flex items-center gap-2"><i className="size-2 rounded-full bg-success" />Memory</span><span className="flex items-center gap-2"><i className="size-2 rounded-full bg-primary" />CPU</span></div></Card></PageSection>
    </>}
    <PageSection><h2 className="font-semibold">Runtime availability</h2><p className="mt-1 text-sm text-muted-foreground">Reported by the local controller on its last check.</p>{runtimeRows.length ? <div className="mt-3 grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-3">{runtimeRows.map(([name, info]) => { const state = info as { available?: boolean; version?: string | null; error?: string | null; image_count?: number; image_directory?: string; acceleration?: string }; const available = state.available ?? Boolean(state.version); return <Card key={name} className="min-w-0 p-4"><div className="flex items-center justify-between gap-3"><h3 className="truncate font-medium">{name === "ubridge" ? "uBridge" : name}</h3><Badge variant={available ? "default" : "destructive"}>{available ? "Available" : "Unavailable"}</Badge></div><p className="mt-2 text-sm text-muted-foreground">{state.version ?? (available ? "Ready" : state.error ?? "Not detected")}</p>{state.image_directory && <p className="mt-2 break-all text-xs text-muted-foreground">Image directory: {state.image_directory}</p>}{typeof state.image_count === "number" && <p className="mt-1 text-xs text-muted-foreground">{state.image_count} local images</p>}{state.acceleration && <p className="mt-1 text-xs text-muted-foreground">Acceleration: {state.acceleration}</p>}{!available && state.error && <p className="mt-2 text-xs text-destructive">{state.error}</p>}</Card>; })}</div> : <Card className="mt-3 p-4 text-sm text-muted-foreground">Runtime availability is loading or unavailable.</Card>}</PageSection>
    <p className="mt-4 text-xs text-muted-foreground">Last sample: {resources?.sampled_at_unix_ms ? new Date(resources.sampled_at_unix_ms).toLocaleTimeString() : "not reported"}</p>
  </PageLayout></PagesShell>;
}
