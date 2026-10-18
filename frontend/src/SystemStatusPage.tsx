import { useEffect, useState } from "react";
import { Activity, Cpu, HardDrive, RefreshCw, Server } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ResourceMonitor, RuntimeInfo } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

type Sample = { at: number; cpu: number | null; memory: number | null };
const bytes = (value?: number | null) => value === null || value === undefined ? "—" : `${(value / 1024 ** 3).toFixed(1)} GiB`;
const percent = (value?: number | null) => value === null || value === undefined ? null : Math.max(0, Math.min(100, value));
const humanError = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

function MetricCard({ label, value, detail, icon: Icon, ratio }: { label: string; value: string; detail: string; icon: LucideIcon; ratio?: number | null }) {
  return <Card className="status-metric"><div className="status-metric-top"><span>{label}</span><Icon size={17} className="text-muted-foreground" /></div><div className="status-metric-value">{value}</div><div className="status-metric-detail">{detail}</div>{ratio !== undefined && ratio !== null && <div className="status-meter"><span style={{ width: `${ratio}%` }} /></div>}</Card>;
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
  return <PagesShell active="System status"><div className="pages-content">
    <PageTitle eyebrow="CONTROLLER" title="System status" description="Live host resources, controller usage, and available runtime services." actions={<Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}><RefreshCw size={15} />Refresh now</Button>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}
    {!resources ? <Card className="pages-empty-state"><Activity className="size-5 text-muted-foreground" /><p>Waiting for resource status from the controller…</p></Card> : <>
      <div className="status-metrics-grid"><MetricCard label="CPU usage" value={resources.host.cpu_usage_percent === null ? "—" : `${resources.host.cpu_usage_percent.toFixed(1)}%`} detail={`${resources.host.cpu_count ?? "—"} logical processors`} icon={Cpu} ratio={percent(resources.host.cpu_usage_percent)} /><MetricCard label="Host memory" value={`${bytes(resources.host.memory_used_bytes)} / ${bytes(resources.host.memory_total_bytes)}`} detail={memoryRatio === null ? "Usage unavailable" : `${memoryRatio.toFixed(1)}% in use`} icon={HardDrive} ratio={percent(memoryRatio)} /><MetricCard label="Controller memory" value={bytes(resources.controller.memory_rss_bytes)} detail={`${resources.controller.thread_count ?? "—"} controller threads`} icon={Server} /><MetricCard label="Load average" value={resources.host.load_average?.map((value) => value.toFixed(2)).join(" · ") ?? "—"} detail="1, 5, and 15 minute averages" icon={Activity} /></div>
      <Card className="status-chart-card"><div className="status-card-heading"><div><h2>Recent resource samples</h2><p>Samples collected while this page is open.</p></div><span className="text-xs text-muted-foreground">5 second polling · {samples.length} samples</span></div><div className="status-sparkline" role="img" aria-label="Recent CPU and memory usage"><div className="status-sparkline-axis"><span>100%</span><span>50%</span><span>0%</span></div><div className="status-sparkline-bars">{samples.map((sample, index) => <div className="status-sparkline-column" key={`${sample.at}-${index}`} title={`CPU ${sample.cpu?.toFixed(1) ?? "—"}%, memory ${sample.memory?.toFixed(1) ?? "—"}%`}><span className="status-sparkline-memory" style={{ height: `${sample.memory ?? 0}%` }} /><span className="status-sparkline-cpu" style={{ height: `${sample.cpu ?? 0}%` }} /></div>)}</div></div><div className="status-sparkline-legend"><span><i className="memory" />Memory</span><span><i className="cpu" />CPU</span></div></Card>
    </>}
    <div className="status-section-heading"><div><h2>Runtime availability</h2><p>Reported by the local controller on its last check.</p></div></div>
    {runtimeRows.length ? <div className="runtime-status-grid">{runtimeRows.map(([name, info]) => { const state = info as { available?: boolean; version?: string | null; error?: string | null; image_count?: number; image_directory?: string; acceleration?: string }; const available = state.available ?? Boolean(state.version); return <Card key={name} className="runtime-status-card"><div className="flex items-center justify-between gap-3"><h3>{name === "ubridge" ? "uBridge" : name}</h3><Badge variant={available ? "default" : "destructive"}>{available ? "Available" : "Unavailable"}</Badge></div><p className="mt-2 text-sm text-muted-foreground">{state.version ?? (available ? "Ready" : state.error ?? "Not detected")}</p>{state.image_directory && <p className="mt-2 break-all text-xs text-muted-foreground">Image directory: {state.image_directory}</p>}{typeof state.image_count === "number" && <p className="mt-1 text-xs text-muted-foreground">{state.image_count} local images</p>}{state.acceleration && <p className="mt-1 text-xs text-muted-foreground">Acceleration: {state.acceleration}</p>}{!available && state.error && <p className="mt-2 text-xs text-destructive">{state.error}</p>}</Card>; })}</div> : <Card className="pages-empty-state">Runtime availability is loading or unavailable.</Card>}
      <p className="status-sampled-at">Last sample: {resources?.sampled_at_unix_ms ? new Date(resources.sampled_at_unix_ms).toLocaleTimeString() : "not reported"}</p>
  </div></PagesShell>;
}
