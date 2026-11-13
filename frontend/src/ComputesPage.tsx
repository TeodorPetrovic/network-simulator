import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Activity, Plus, RefreshCw, Server } from "lucide-react";
import type { Compute, RuntimeInfo } from "./api";
import { api } from "./api";
import { PagesShell, PageLayout, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useConfirm } from "@/components/confirm-dialog";

const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

export function ComputesPage() {
  const navigate = useNavigate();
  const [computes, setComputes] = useState<Compute[]>([]);
  const [runtimes, setRuntimes] = useState<RuntimeInfo | null>(null);
  const [computeError, setComputeError] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<Compute | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [formError, setFormError] = useState("");
  const [confirm, confirmDialog] = useConfirm();
  const [form, setForm] = useState({ name: "", protocol: "http", host: "", port: "3080", user: "", password: "", compute_id: "" });

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
  function openForm(compute?: Compute) {
    setFormOpen(true);
    setEditing(compute ?? null); setFormError("");
    setForm({ name: compute?.name ?? "", protocol: compute?.protocol ?? "http", host: compute?.host ?? "", port: String(compute?.port ?? 3080), user: String(compute?.user ?? ""), password: "", compute_id: compute?.compute_id ?? "" });
  }
  async function saveCompute(event: React.FormEvent) {
    event.preventDefault(); setFormError("");
    const payload = { ...form, port: Number(form.port) };
    try { if (editing) await api.updateCompute(editing.compute_id, payload); else await api.createCompute(payload); setEditing(null); setFormOpen(false); await refresh(); }
    catch (reason) { setFormError(errorMessage(reason)); }
  }
  async function removeCompute(compute: Compute) {
    if (!(await confirm({ title: "Delete compute?", description: `Delete compute ${compute.name}?`, confirmLabel: "Delete", destructive: true }))) return;
    try { await api.deleteCompute(compute.compute_id); await refresh(); } catch (reason) { setComputeError(errorMessage(reason)); }
  }

  return <PagesShell active="Servers"><PageLayout>{confirmDialog}
    <PageTitle eyebrow="CONTROLLER" title="Servers and computes" description="Inspect computes registered with this controller and its local runtime services." actions={<Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}><RefreshCw size={15} />Refresh</Button>} />
    {error && <div role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">{error}</div>}
    <Card className="overflow-hidden"><div className="flex items-center justify-between gap-4 p-4"><div><h2 className="text-base font-semibold">Registered computes</h2><p className="mt-0.5 text-sm text-muted-foreground">Compute endpoints reported by the controller.</p></div><Button size="sm" onClick={() => openForm()}><Plus size={15} />Add compute</Button></div>{computeError && <div role="alert" className="m-4 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{computeError}</div>}<Table><TableHeader><TableRow><TableHead>ID</TableHead><TableHead>Name</TableHead><TableHead>Location</TableHead><TableHead>Host</TableHead><TableHead>Port</TableHead><TableHead>Connection</TableHead><TableHead>Actions</TableHead></TableRow></TableHeader><TableBody>{busy && !computes.length ? <TableRow><TableCell colSpan={7} className="py-8 text-center text-muted-foreground">Loading computes…</TableCell></TableRow> : computes.length ? computes.map((compute) => <TableRow key={compute.compute_id}><TableCell className="font-mono text-xs">{compute.compute_id}</TableCell><TableCell className="font-medium">{compute.name}</TableCell><TableCell>{compute.compute_id === "local" ? "Local" : "Remote"}</TableCell><TableCell>{compute.host ?? "—"}</TableCell><TableCell>{compute.port ?? "—"}</TableCell><TableCell><Badge variant={compute.connected ? "default" : "secondary"}>{compute.connected ? "Connected" : "Disconnected"}</Badge>{typeof compute.last_error === "string" && <div className="mt-1 max-w-56 text-xs text-destructive">{compute.last_error}</div>}</TableCell><TableCell><div className="flex gap-2"><Button size="sm" variant="outline" disabled={compute.compute_id === "local"} onClick={() => openForm(compute)}>Edit</Button><Button size="sm" variant="outline" disabled={compute.compute_id === "local"} onClick={() => void removeCompute(compute)}>Delete</Button></div></TableCell></TableRow>) : !computeError && <TableRow><TableCell colSpan={7} className="py-8 text-center text-muted-foreground">No registered computes were returned.</TableCell></TableRow>}</TableBody></Table></Card>
    <Dialog open={formOpen} onOpenChange={(open) => { if (!open) { setEditing(null); setFormError(""); setFormOpen(false); } }}><DialogContent aria-label={editing ? "Edit compute" : "Add compute"}><DialogHeader><DialogTitle>{editing ? "Edit compute" : "Add compute"}</DialogTitle><DialogDescription>Configure the compute endpoint registered with the controller.</DialogDescription></DialogHeader><form onSubmit={saveCompute} className="space-y-3">{formError && <p role="alert" className="text-sm text-destructive">{formError}</p>}{([ ["Name","name"], ["Host","host"], ["Port","port"], ["Protocol","protocol"], ["User","user"], ["Password","password"], ["Compute ID","compute_id"] ] as const).map(([label,key]) => <Label key={key} className="grid gap-1 text-sm">{label}<Input required={key === "name" || key === "host" || key === "port"} type={key === "password" ? "password" : key === "port" ? "number" : "text"} min={key === "port" ? 1 : undefined} max={key === "port" ? 65535 : undefined} value={form[key]} onChange={(event) => setForm({ ...form, [key]: event.target.value })} /></Label>)}<DialogFooter><Button type="button" variant="outline" onClick={() => { setEditing(null); setFormError(""); setFormOpen(false); }}>Cancel</Button><Button type="submit">Save</Button></DialogFooter></form></DialogContent></Dialog>
    <div className="mt-8 flex items-center justify-between gap-4"><div><h2 className="text-base font-semibold">Local runtime status</h2><p className="mt-0.5 text-sm text-muted-foreground">The current controller process and the binaries it can use.</p></div><Button variant="outline" size="sm" onClick={() => void navigate({ to: "/system-status" as never })}><Activity size={14} />System status</Button></div>
    {runtimes ? <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">{Object.entries(runtimes).map(([name, value]) => { const runtime = value as { available?: boolean; version?: string | null; error?: string | null; image_count?: number }; return <Card className="p-4" key={name}><div className="flex items-center justify-between gap-2"><h3 className="text-sm font-semibold">{name}</h3><Badge variant={runtime.available ? "default" : "secondary"}>{runtime.available ? "Available" : "Unavailable"}</Badge></div><p className="mt-2 text-sm text-muted-foreground">{runtime.version ?? runtime.error ?? "No version reported"}</p>{runtime.image_count !== undefined && <p className="mt-1 text-xs text-muted-foreground">{runtime.image_count} local images</p>}</Card>; })}</div> : <Card className="mt-4 flex items-center gap-2 p-4 text-muted-foreground"><Server size={18} />Local runtime status is unavailable.</Card>}
  </PageLayout></PagesShell>;
}
