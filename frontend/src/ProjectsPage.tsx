import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowDown, ArrowUp, BarChart3, Copy, Download, FileUp, FolderOpen, Pencil, Plus, Search, Trash2 } from "lucide-react";
import type { Project, ProjectStatistics } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type SortColumn = "name" | "status" | "created_at";
type ProjectDraft = {
  name: string;
  scene_width: string;
  scene_height: string;
  grid_size: string;
  drawing_grid_size: string;
  auto_open: boolean;
  auto_start: boolean;
  auto_close: boolean;
  show_interface_labels: boolean;
  variables: Array<{ name: string; value: string }>;
};

const blankDraft = (project?: Project): ProjectDraft => ({
  name: project?.name ?? "",
  scene_width: String(project?.scene_width ?? 2000),
  scene_height: String(project?.scene_height ?? 1000),
  grid_size: String(project?.grid_size ?? 75),
  drawing_grid_size: String(project?.drawing_grid_size ?? 25),
  auto_open: Boolean(project?.auto_open),
  auto_start: Boolean(project?.auto_start),
  auto_close: Boolean(project?.auto_close),
  show_interface_labels: Boolean(project?.show_interface_labels),
  variables: Array.isArray(project?.variables) ? project.variables.map(({ name, value }) => ({ name, value })) : [],
});

function message(reason: unknown) { return reason instanceof Error ? reason.message : "Request failed"; }
function projectUuid() {
  return globalThis.crypto?.randomUUID?.() ?? `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`.replace(/[xy]/g, (c) => {
    const r = Math.random() * 16 | 0;
    return (c === "x" ? r : (r & 0x3 | 0x8)).toString(16);
  });
}

export function ProjectsPage() {
  const navigate = useNavigate();
  const [projects, setProjects] = useState<Project[]>([]);
  const [busy, setBusy] = useState(true);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<{ column: SortColumn; direction: "asc" | "desc" }>({ column: "name", direction: "asc" });
  const [newName, setNewName] = useState("");
  const [importOpen, setImportOpen] = useState(false);
  const [importFile, setImportFile] = useState<File | null>(null);
  const [importName, setImportName] = useState("");
  const [duplicateTarget, setDuplicateTarget] = useState<Project | null>(null);
  const [duplicateName, setDuplicateName] = useState("");
  const [settingsTarget, setSettingsTarget] = useState<Project | null>(null);
  const [draft, setDraft] = useState<ProjectDraft>(blankDraft());
  const [readmeTarget, setReadmeTarget] = useState<Project | null>(null);
  const [readme, setReadme] = useState("");
  const [statisticsTarget, setStatisticsTarget] = useState<Project | null>(null);
  const [statistics, setStatistics] = useState<ProjectStatistics | null>(null);
  const [detailsError, setDetailsError] = useState("");

  async function refresh() {
    setBusy(true); setError("");
    try { setProjects(await api.listProjects()); }
    catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);

  const visibleProjects = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return projects.filter((project) => !needle || project.name.toLocaleLowerCase().includes(needle))
      .sort((left, right) => {
        const a = String(left[sort.column] ?? "").toLocaleLowerCase();
        const b = String(right[sort.column] ?? "").toLocaleLowerCase();
        return (a.localeCompare(b) || left.project_id.localeCompare(right.project_id)) * (sort.direction === "asc" ? 1 : -1);
      });
  }, [projects, query, sort]);

  function setSortColumn(column: SortColumn) {
    setSort((current) => ({ column, direction: current.column === column && current.direction === "asc" ? "desc" : "asc" }));
  }

  async function createProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!newName.trim()) return;
    setWorking(true); setError("");
    try { const project = await api.createProject(newName.trim()); await navigate({ to: "/projects/$projectId", params: { projectId: project.project_id } }); }
    catch (reason) { setError(message(reason)); }
    finally { setWorking(false); }
  }

  async function importProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!importFile || !importName.trim()) return;
    setWorking(true); setError("");
    try {
      const project = await api.importProject(projectUuid(), importName.trim(), importFile);
      setProjects((current) => [...current, project]); setImportOpen(false); setImportFile(null); setImportName("");
      setNotice(`Imported “${project.name}”.`);
    } catch (reason) { setError(message(reason)); }
    finally { setWorking(false); }
  }

  async function duplicateProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!duplicateTarget || !duplicateName.trim()) return;
    setWorking(true); setError("");
    try { const copy = await api.duplicateProject(duplicateTarget.project_id, duplicateName.trim()); setProjects((current) => [...current, copy]); setDuplicateTarget(null); setNotice(`Duplicated as “${copy.name}”.`); }
    catch (reason) { setError(message(reason)); }
    finally { setWorking(false); }
  }

  async function exportProject(project: Project) {
    setError(""); setWorking(true);
    try {
      const blob = await api.exportProject(project.project_id);
      const href = URL.createObjectURL(blob); const anchor = document.createElement("a");
      anchor.href = href; anchor.download = `${project.name}.gns3project`; anchor.click(); URL.revokeObjectURL(href);
    } catch (reason) { setError(`Export unavailable: ${message(reason)}`); }
    finally { setWorking(false); }
  }

  async function deleteProject(project: Project) {
    if (!window.confirm(`Delete “${project.name}” and its topology?`)) return;
    setWorking(true); setError("");
    try { await api.deleteProject(project.project_id); setProjects((current) => current.filter((item) => item.project_id !== project.project_id)); }
    catch (reason) { setError(message(reason)); }
    finally { setWorking(false); }
  }

  function openSettings(project: Project) { setSettingsTarget(project); setDraft(blankDraft(project)); setDetailsError(""); }
  async function saveSettings(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!settingsTarget) return;
    const variables = draft.variables.map(({ name, value }) => ({ name: name.trim(), value })).filter(({ name }) => name);
    const patch = {
      name: draft.name.trim(), scene_width: Number(draft.scene_width), scene_height: Number(draft.scene_height),
      grid_size: Number(draft.grid_size), drawing_grid_size: Number(draft.drawing_grid_size),
      auto_open: draft.auto_open, auto_start: draft.auto_start, auto_close: draft.auto_close,
      show_interface_labels: draft.show_interface_labels, variables,
    };
    setWorking(true); setDetailsError("");
    try { const updated = await api.updateProject(settingsTarget.project_id, patch); setProjects((current) => current.map((item) => item.project_id === updated.project_id ? updated : item)); setSettingsTarget(null); }
    catch (reason) { setDetailsError(`Project settings could not be saved: ${message(reason)}`); }
    finally { setWorking(false); }
  }

  async function openReadme(project: Project) {
    setReadmeTarget(project); setReadme(""); setDetailsError("");
    try { const result = await api.projectReadme(project.project_id); setReadme(result.content); }
    catch (reason) { setDetailsError(`Readme is unavailable: ${message(reason)}`); }
  }
  async function saveReadme(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!readmeTarget) return;
    setWorking(true); setDetailsError("");
    try { await api.updateProjectReadme(readmeTarget.project_id, readme); setReadmeTarget(null); setNotice("Project readme saved."); }
    catch (reason) { setDetailsError(`Readme save is unavailable: ${message(reason)}`); }
    finally { setWorking(false); }
  }
  async function openStatistics(project: Project) {
    setStatisticsTarget(project); setStatistics(null); setDetailsError("");
    try { setStatistics(await api.projectStatistics(project.project_id)); }
    catch (reason) { setDetailsError(`Project statistics are unavailable: ${message(reason)}`); }
  }

  const checkbox = (key: "auto_open" | "auto_start" | "auto_close" | "show_interface_labels", text: string) => <label key={key} className="pages-checkbox"><input type="checkbox" checked={draft[key]} onChange={(event) => setDraft((current) => ({ ...current, [key]: event.target.checked }))} />{text}</label>;
  const field = (key: "name" | "scene_width" | "scene_height" | "grid_size" | "drawing_grid_size", label: string, type = "text") => <label key={key} className="grid gap-1.5"><Label htmlFor={`project-${key}`}>{label}</Label><Input id={`project-${key}`} type={type} min={type === "number" ? 1 : undefined} value={draft[key]} onChange={(event) => setDraft((current) => ({ ...current, [key]: event.target.value }))} /></label>;

  return <PagesShell active="Projects"><div className="pages-content">
    <PageTitle eyebrow="LAB MANAGEMENT" title="Projects" description="Create, organize, import, and inspect network labs." actions={<Button variant="outline" onClick={() => void navigate({ to: "/appliances" as never })}>Appliance manager</Button>} />
    {error && <div role="alert" className="pages-alert">{error}<Button size="sm" variant="ghost" onClick={() => setError("")}>Dismiss</Button></div>}
    {notice && <div role="status" className="pages-notice">{notice}</div>}

    <div className="projects-toolbar"><form className="projects-create" onSubmit={(event) => void createProject(event)}><Input aria-label="New project name" placeholder="Project name" value={newName} onChange={(event) => setNewName(event.target.value)} /><Button type="submit" disabled={working || !newName.trim()}><Plus size={16} />Create project</Button></form><Button variant="outline" onClick={() => { setImportOpen(true); setError(""); }}><FileUp size={16} />Import .gns3project</Button><label className="projects-search"><Search size={15} /><Input aria-label="Search projects by name" placeholder="Search by name…" value={query} onChange={(event) => setQuery(event.target.value)} /></label></div>

    <Card className="projects-table-card"><Table><TableHeader><TableRow>
      {([ ["name", "Project"], ["status", "Status"], ["created_at", "Created"] ] as Array<[SortColumn, string]>).map(([key, label]) => <TableHead key={key}><button className="projects-sort" onClick={() => setSortColumn(key)}>{label}{sort.column === key && (sort.direction === "asc" ? <ArrowUp size={13} /> : <ArrowDown size={13} />)}</button></TableHead>)}
      <TableHead>Actions</TableHead>
    </TableRow></TableHeader><TableBody>
      {busy ? <TableRow><TableCell colSpan={4} className="py-10 text-center text-muted-foreground">Loading projects…</TableCell></TableRow> : visibleProjects.length === 0 ? <TableRow><TableCell colSpan={4} className="py-10 text-center text-muted-foreground">{query ? "No projects match this search." : "No projects yet. Create one or import a .gns3project archive."}</TableCell></TableRow> : visibleProjects.map((project) => <TableRow key={project.project_id}>
        <TableCell><div className="flex items-center gap-2"><FolderOpen size={16} className="text-primary" /><div><div className="font-medium">{project.name}</div><div className="text-xs text-muted-foreground">{project.project_id}</div></div></div></TableCell>
        <TableCell><Badge variant={project.status === "opened" ? "default" : "secondary"}>{project.status ?? "closed"}</Badge></TableCell>
        <TableCell>{typeof project.created_at === "string" ? new Date(project.created_at).toLocaleString() : "—"}</TableCell>
        <TableCell><div className="flex flex-wrap justify-end gap-1">
          <Button size="sm" onClick={() => void navigate({ to: "/projects/$projectId", params: { projectId: project.project_id } })}><FolderOpen size={14} />Open project</Button>
          <Button size="icon-sm" variant="ghost" title="Project settings" aria-label={`Settings for ${project.name}`} onClick={() => openSettings(project)}><Pencil size={15} /></Button>
          <Button size="icon-sm" variant="ghost" title="Project readme" aria-label={`Readme for ${project.name}`} onClick={() => void openReadme(project)}><span className="text-xs font-semibold">R</span></Button>
          <Button size="icon-sm" variant="ghost" title="Project statistics" aria-label={`Statistics for ${project.name}`} onClick={() => void openStatistics(project)}><BarChart3 size={15} /></Button>
          <Button size="icon-sm" variant="ghost" title="Export project" aria-label={`Export ${project.name}`} disabled={working} onClick={() => void exportProject(project)}><Download size={15} /></Button>
          <Button size="icon-sm" variant="ghost" title="Duplicate project" aria-label={`Duplicate ${project.name}`} onClick={() => { setDuplicateTarget(project); setDuplicateName(`${project.name} copy`); }}><Copy size={15} /></Button>
          <Button size="icon-sm" variant="ghost" title="Delete project" aria-label={`Delete ${project.name}`} onClick={() => void deleteProject(project)}><Trash2 size={15} /></Button>
        </div></TableCell>
      </TableRow>)}
    </TableBody></Table></Card>

    <Dialog open={importOpen} onOpenChange={setImportOpen}><DialogContent><DialogHeader><DialogTitle>Import project</DialogTitle><DialogDescription>Import a GNS3 portable project archive (.gns3project or .gns3p).</DialogDescription></DialogHeader><form className="grid gap-4" onSubmit={(event) => void importProject(event)}><label className="grid gap-1.5"><Label htmlFor="project-import-file">Project archive</Label><Input id="project-import-file" type="file" accept=".gns3project,.gns3p" onChange={(event) => { const file = event.target.files?.[0] ?? null; setImportFile(file); if (file && !importName) setImportName(file.name.replace(/\.(gns3project|gns3p)$/i, "")); }} /></label><label className="grid gap-1.5"><Label htmlFor="project-import-name">Project name</Label><Input id="project-import-name" value={importName} onChange={(event) => setImportName(event.target.value)} required /></label>{error && <p role="alert" className="text-sm text-destructive">{error}</p>}<DialogFooter><Button type="button" variant="ghost" onClick={() => setImportOpen(false)}>Cancel</Button><Button type="submit" disabled={working || !importFile || !importName.trim()}>{working ? "Importing…" : "Import project"}</Button></DialogFooter></form></DialogContent></Dialog>

    <Dialog open={Boolean(duplicateTarget)} onOpenChange={(open) => !open && setDuplicateTarget(null)}><DialogContent><DialogHeader><DialogTitle>Duplicate project</DialogTitle><DialogDescription>Create a copy of {duplicateTarget?.name ?? "this project"}.</DialogDescription></DialogHeader><form className="grid gap-4" onSubmit={(event) => void duplicateProject(event)}><label className="grid gap-1.5"><Label htmlFor="duplicate-project-name">New project name</Label><Input id="duplicate-project-name" value={duplicateName} onChange={(event) => setDuplicateName(event.target.value)} required /></label>{error && <p role="alert" className="text-sm text-destructive">{error}</p>}<DialogFooter><Button type="button" variant="ghost" onClick={() => setDuplicateTarget(null)}>Cancel</Button><Button type="submit" disabled={working || !duplicateName.trim()}>{working ? "Duplicating…" : "Duplicate"}</Button></DialogFooter></form></DialogContent></Dialog>

    <Dialog open={Boolean(settingsTarget)} onOpenChange={(open) => !open && setSettingsTarget(null)}><DialogContent className="pages-dialog-wide"><DialogHeader><DialogTitle>Project settings</DialogTitle><DialogDescription>Scene, startup, and project variables for {settingsTarget?.name ?? "this project"}.</DialogDescription></DialogHeader><form onSubmit={(event) => void saveSettings(event)}><Tabs defaultValue="general"><TabsList><TabsTrigger value="general">General</TabsTrigger><TabsTrigger value="variables">Global variables</TabsTrigger></TabsList><TabsContent value="general" className="grid gap-4 pt-4"><div className="grid gap-3 sm:grid-cols-2">{field("name", "Project name")}{field("scene_width", "Scene width (px)", "number")}{field("scene_height", "Scene height (px)", "number")}{field("grid_size", "Node grid size", "number")}{field("drawing_grid_size", "Drawing grid size", "number")}</div><div className="grid gap-2">{checkbox("auto_open", "Open this project in the background when the controller starts")}{checkbox("auto_start", "Start all nodes when this project is opened")}{checkbox("auto_close", "Leave this project running in the background after closing")}{checkbox("show_interface_labels", "Show interface labels at start")}</div></TabsContent><TabsContent value="variables" className="grid gap-3 pt-4"><div className="flex items-center justify-between"><div><h3 className="text-sm font-medium">Project variables</h3><p className="text-xs text-muted-foreground">Variables available to project node and appliance settings.</p></div><Button type="button" variant="outline" size="sm" onClick={() => setDraft((current) => ({ ...current, variables: [...current.variables, { name: "", value: "" }] }))}><Plus size={14} />Add variable</Button></div>{draft.variables.map((variable, index) => <div key={index} className="grid items-end gap-2 sm:grid-cols-[1fr_1fr_auto]"><label className="grid gap-1"><Label htmlFor={`variable-name-${index}`}>Name</Label><Input id={`variable-name-${index}`} value={variable.name} onChange={(event) => setDraft((current) => ({ ...current, variables: current.variables.map((item, row) => row === index ? { ...item, name: event.target.value } : item) }))} /></label><label className="grid gap-1"><Label htmlFor={`variable-value-${index}`}>Value</Label><Input id={`variable-value-${index}`} value={variable.value} onChange={(event) => setDraft((current) => ({ ...current, variables: current.variables.map((item, row) => row === index ? { ...item, value: event.target.value } : item) }))} /></label><Button type="button" variant="ghost" size="icon" aria-label={`Remove variable ${index + 1}`} onClick={() => setDraft((current) => ({ ...current, variables: current.variables.filter((_, row) => row !== index) }))}><Trash2 size={15} /></Button></div>)}</TabsContent></Tabs>{detailsError && <p role="alert" className="mt-3 text-sm text-destructive">{detailsError}</p>}<DialogFooter className="mt-4"><Button type="button" variant="ghost" onClick={() => setSettingsTarget(null)}>Cancel</Button><Button type="submit" disabled={working}>{working ? "Saving…" : "Apply"}</Button></DialogFooter></form></DialogContent></Dialog>

    <Dialog open={Boolean(readmeTarget)} onOpenChange={(open) => !open && setReadmeTarget(null)}><DialogContent><DialogHeader><DialogTitle>Project readme</DialogTitle><DialogDescription>Readme file for {readmeTarget?.name ?? "this project"}.</DialogDescription></DialogHeader><form className="grid gap-3" onSubmit={(event) => void saveReadme(event)}><Textarea aria-label="Project readme content" value={readme} onChange={(event) => setReadme(event.target.value)} rows={12} />{detailsError && <p role="alert" className="text-sm text-destructive">{detailsError}</p>}<DialogFooter><Button type="button" variant="ghost" onClick={() => setReadmeTarget(null)}>Close</Button><Button type="submit" disabled={working || Boolean(detailsError)}>{working ? "Saving…" : "Save readme"}</Button></DialogFooter></form></DialogContent></Dialog>

    <Dialog open={Boolean(statisticsTarget)} onOpenChange={(open) => !open && setStatisticsTarget(null)}><DialogContent><DialogHeader><DialogTitle>Project statistics</DialogTitle><DialogDescription>Statistics reported by the controller for {statisticsTarget?.name ?? "this project"}.</DialogDescription></DialogHeader>{detailsError ? <p role="alert" className="text-sm text-destructive">{detailsError}</p> : statistics ? <div className="grid grid-cols-2 gap-3">{Object.entries(statistics).map(([key, value]) => <div key={key} className="rounded-md border p-3"><div className="text-xs capitalize text-muted-foreground">{key.replaceAll("_", " ")}</div><div className="mt-1 text-lg font-semibold">{typeof value === "number" || typeof value === "string" ? value : JSON.stringify(value)}</div></div>)}</div> : <p className="text-sm text-muted-foreground">Loading statistics…</p>}<DialogFooter><Button variant="outline" onClick={() => setStatisticsTarget(null)}>Close</Button></DialogFooter></DialogContent></Dialog>
  </div></PagesShell>;
}
