import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Activity, RefreshCw, Server } from "lucide-react";
import type { Compute, RuntimeInfo } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

export function ComputesPage() {
  const navigate = useNavigate();
  const [computes, setComputes] = useState<Compute[]>([]);
  const [runtimes, setRuntimes] = useState<RuntimeInfo | null>(null);
  const [computeError, setComputeError] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true); setError("");
    const [computeResult, runtimeResult] = await Promise.allSettled([api.computes(), api.runtimes()]);
    if (computeResult.status === "fulfilled") { setComputes(computeResult.value); setComputeError(""); }
    else { setComputes([]); setComputeError(`Compute management unavailable: ${errorMessage(computeResult.reason)}`); }
    if (runtimeResult.status === "fulfilled") setRuntimes(runtimeResult.value);
    else setError(`Local runtime status unavailable: ${errorMessage(runtimeResult.reason)}`);
    setBusy(false);
  }
  useEffect(() => { void refresh(); }, []);

  return <PagesShell active="Servers"><div className="pages-content">
    <PageTitle eyebrow="CONTROLLER" title="Servers and computes" description="Inspect computes registered with this controller and its local runtime services." actions={<Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}><RefreshCw size={15} />Refresh</Button>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}
    <Card className="compute-table-card"><div className="pages-card-heading"><div><h2>Registered computes</h2><p>Compute endpoints reported by the controller.</p></div></div>{computeError && <div role="alert" className="pages-inline-alert">{computeError}</div>}<Table><TableHeader><TableRow><TableHead>ID</TableHead><TableHead>Name</TableHead><TableHead>Location</TableHead><TableHead>Host</TableHead><TableHead>Port</TableHead><TableHead>Connection</TableHead></TableRow></TableHeader><TableBody>{busy && !computes.length ? <TableRow><TableCell colSpan={6} className="py-8 text-center text-muted-foreground">Loading computes…</TableCell></TableRow> : computes.length ? computes.map((compute) => <TableRow key={compute.compute_id}><TableCell className="font-mono text-xs">{compute.compute_id}</TableCell><TableCell className="font-medium">{compute.name}</TableCell><TableCell>{compute.compute_id === "local" ? "Local" : "Remote"}</TableCell><TableCell>{compute.host ?? "—"}</TableCell><TableCell>{compute.port ?? "—"}</TableCell><TableCell><Badge variant={compute.connected ? "default" : "secondary"}>{compute.connected ? "Connected" : "Disconnected"}</Badge>{typeof compute.last_error === "string" && <div className="mt-1 max-w-56 text-xs text-destructive">{compute.last_error}</div>}</TableCell></TableRow>) : !computeError && <TableRow><TableCell colSpan={6} className="py-8 text-center text-muted-foreground">No registered computes were returned.</TableCell></TableRow>}</TableBody></Table></Card>
    <div className="pages-card-heading mt-7"><div><h2>Local runtime status</h2><p>The current controller process and the binaries it can use.</p></div><Button variant="outline" size="sm" onClick={() => void navigate({ to: "/system-status" as never })}><Activity size={14} />System status</Button></div>
    {runtimes ? <div className="runtime-status-grid">{Object.entries(runtimes).map(([name, value]) => { const runtime = value as { available?: boolean; version?: string | null; error?: string | null; image_count?: number }; return <Card className="runtime-status-card" key={name}><div className="flex items-center justify-between gap-2"><h3>{name}</h3><Badge variant={runtime.available ? "default" : "secondary"}>{runtime.available ? "Available" : "Unavailable"}</Badge></div><p className="mt-2 text-sm text-muted-foreground">{runtime.version ?? runtime.error ?? "No version reported"}</p>{runtime.image_count !== undefined && <p className="mt-1 text-xs text-muted-foreground">{runtime.image_count} local images</p>}</Card>; })}</div> : <Card className="pages-empty-state"><Server size={18} />Local runtime status is unavailable.</Card>}
    <p className="compute-note">This controller exposes read-only compute listing and details. Registering or removing remote computes, and local server start/stop, need controller routes that are not present yet.</p>
  </div></PagesShell>;
}
