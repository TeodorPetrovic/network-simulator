import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, Plus, RotateCcw, Search, Trash2 } from "lucide-react";
import type { ProjectSnapshot } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";
export function SnapshotsPage({ projectId }: { projectId: string }) {
  const navigate = useNavigate();
  const [snapshots, setSnapshots] = useState<ProjectSnapshot[]>([]);
  const [projectName, setProjectName] = useState(projectId);
  const [filter, setFilter] = useState("");
  const [sortAsc, setSortAsc] = useState(true);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");

  async function refresh() {
    setLoading(true); setError("");
    try { const [items, topology] = await Promise.all([api.snapshots(projectId), api.topology(projectId).catch(() => null)]); setSnapshots(items); if (topology) setProjectName(topology.project.name); }
    catch (reason) { setError(`Snapshots are unavailable: ${errorMessage(reason)}`); }
    finally { setLoading(false); }
  }
  useEffect(() => { void refresh(); }, [projectId]);
  const rows = useMemo(() => snapshots.filter((item) => item.name.toLocaleLowerCase().includes(filter.toLocaleLowerCase())).sort((a, b) => (a.name.localeCompare(b.name) || String(a.created_at ?? "").localeCompare(String(b.created_at ?? ""))) * (sortAsc ? 1 : -1)), [snapshots, filter, sortAsc]);

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!name.trim()) return;
    setBusy(true); setError("");
    try { await api.createSnapshot(projectId, name.trim()); setName(""); setCreateOpen(false); await refresh(); }
    catch (reason) { setError(`Snapshot creation is unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function restore(snapshot: ProjectSnapshot) {
    if (!window.confirm(`Restore “${snapshot.name}”? The current project topology will be replaced.`)) return;
    setBusy(true); setError("");
    try { await api.restoreSnapshot(projectId, snapshot.snapshot_id); await navigate({ to: "/projects/$projectId", params: { projectId } }); }
    catch (reason) { setError(`Snapshot restore is unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function remove(snapshot: ProjectSnapshot) {
    if (!window.confirm(`Delete snapshot “${snapshot.name}”?`)) return;
    setBusy(true); setError("");
    try { await api.deleteSnapshot(projectId, snapshot.snapshot_id); setSnapshots((current) => current.filter((item) => item.snapshot_id !== snapshot.snapshot_id)); }
    catch (reason) { setError(`Snapshot deletion is unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }

  return <PagesShell active="Projects"><div className="pages-content"><PageTitle eyebrow="PROJECT ARCHIVE" title="Snapshots" description={`Saved topology states for ${projectName}.`} actions={<><Button variant="outline" onClick={() => void navigate({ to: "/projects/$projectId", params: { projectId } })}><ArrowLeft size={15} />Back to project</Button><Button onClick={() => setCreateOpen(true)}><Plus size={15} />Create snapshot</Button></>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}
    <div className="projects-toolbar"><label className="projects-search"><Search size={15} /><Input aria-label="Filter snapshots" placeholder="Filter snapshots…" value={filter} onChange={(event) => setFilter(event.target.value)} /></label></div>
    <Card className="projects-table-card"><Table><TableHeader><TableRow><TableHead><button className="projects-sort" onClick={() => setSortAsc((current) => !current)}>Name</button></TableHead><TableHead>Date</TableHead><TableHead>Actions</TableHead></TableRow></TableHeader><TableBody>{loading ? <TableRow><TableCell colSpan={3} className="py-10 text-center text-muted-foreground">Loading snapshots…</TableCell></TableRow> : rows.length ? rows.map((snapshot) => <TableRow key={snapshot.snapshot_id}><TableCell><div className="font-medium">{snapshot.name}</div><div className="text-xs text-muted-foreground">{snapshot.snapshot_id}</div></TableCell><TableCell>{typeof snapshot.created_at === "number" && Number.isFinite(snapshot.created_at) ? new Date(snapshot.created_at * 1000).toLocaleString() : "—"}</TableCell><TableCell><div className="flex justify-end gap-1"><Button size="sm" variant="outline" disabled={busy} onClick={() => void restore(snapshot)}><RotateCcw size={14} />Restore</Button><Button size="icon-sm" variant="ghost" aria-label={`Delete snapshot ${snapshot.name}`} disabled={busy} onClick={() => void remove(snapshot)}><Trash2 size={15} /></Button></div></TableCell></TableRow>) : <TableRow><TableCell colSpan={3} className="py-10 text-center text-muted-foreground">{filter ? "No snapshots match this filter." : "No snapshots have been saved for this project."}</TableCell></TableRow>}</TableBody></Table></Card>
    <Dialog open={createOpen} onOpenChange={setCreateOpen}><DialogContent><DialogHeader><DialogTitle>Create snapshot</DialogTitle><DialogDescription>Save the current topology state for later restoration.</DialogDescription></DialogHeader><form className="grid gap-3" onSubmit={(event) => void create(event)}><label className="grid gap-1.5"><Label htmlFor="snapshot-name">Name</Label><Input id="snapshot-name" value={name} onChange={(event) => setName(event.target.value)} required /></label><DialogFooter><Button type="button" variant="ghost" onClick={() => setCreateOpen(false)}>Cancel</Button><Button type="submit" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create snapshot"}</Button></DialogFooter></form></DialogContent></Dialog>
  </div></PagesShell>;
}
