import type { FormEvent } from "react";
import { FolderOpen, Network, Plus, Search, Trash2 } from "lucide-react";
import type { Project } from "./api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type ProjectsPageProps = {
  projects: Project[];
  filteredProjects: Project[];
  busy: boolean;
  filter: string;
  projectName: string;
  creatingProject: boolean;
  onFilterChange: (value: string) => void;
  onProjectNameChange: (value: string) => void;
  onCreateProject: (event: FormEvent<HTMLFormElement>) => void;
  onNavigate: (id: string | null) => void;
  onOpenAppliances?: () => void;
  onOpenMarketplace: () => void;
  onDeleteProject: (project: Project) => void;
};

export function ProjectsPage({ projects, filteredProjects, busy, filter, projectName, creatingProject, onFilterChange, onProjectNameChange, onCreateProject, onNavigate, onOpenAppliances, onOpenMarketplace, onDeleteProject }: ProjectsPageProps) {
  return (
<section className="projects-page">
            <div className="projects-heading">
              <div><div className="sim-section-label">LAB MANAGEMENT</div><h1>Projects</h1><p>Open a project or create a network lab.</p></div>
              <div className="flex flex-wrap gap-2"><Button variant="outline" onClick={onOpenAppliances}><Network />Appliance manager</Button><Button variant="outline" onClick={onOpenMarketplace}><Search />Image catalog</Button></div>
            </div>
            <Card className="sim-create-project">
              <div className="sim-project-create-icon"><Plus /></div>
              <div className="min-w-0 flex-1"><h2>New project</h2><p>Create an empty topology, then add devices from the global image library.</p></div>
              <form className="sim-create-form" onSubmit={onCreateProject}>
                <Input id="new-project-name" aria-label="New project name" placeholder="e.g. Branch office network" value={projectName} onChange={(event) => onProjectNameChange(event.target.value)} />
                <Button type="submit" disabled={!projectName.trim() || creatingProject}><Plus />{creatingProject ? "Creating…" : "Create project"}</Button>
              </form>
            </Card>
            <div className="sim-project-list-heading"><div><h2>All projects <Badge variant="secondary">{projects.length}</Badge></h2><p>Projects are stored by the local controller.</p></div><label className="relative w-64"><Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground" /><Input aria-label="Filter projects" placeholder="Filter projects…" value={filter} onChange={(event) => onFilterChange(event.target.value)} className="pl-9" /></label></div>
            {busy ? <Card className="sim-empty-state">Loading projects…</Card> : filteredProjects.length ? <Card className="sim-project-table-card"><Table>
              <TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Status</TableHead><TableHead>Location</TableHead><TableHead className="w-32 text-right">Actions</TableHead></TableRow></TableHeader>
              <TableBody>{filteredProjects.map((item) => <TableRow key={item.project_id}>
                <TableCell><Button variant="ghost" className="sim-project-link h-auto max-w-full justify-start px-0 py-1 text-left" onClick={() => onNavigate(item.project_id)}><span className="sim-project-table-icon"><FolderOpen size={17} /></span><span className="truncate">{item.name}</span></Button></TableCell>
                <TableCell><Badge variant="outline" className="capitalize">{item.status ?? "closed"}</Badge></TableCell>
                <TableCell className="text-muted-foreground">Local controller</TableCell>
                <TableCell><div className="flex justify-end gap-1"><Button variant="outline" size="sm" onClick={() => onNavigate(item.project_id)}>Open project</Button><Button variant="ghost" size="icon-sm" onClick={() => onDeleteProject(item)} aria-label={`Delete ${item.name}`} title="Delete project"><Trash2 /></Button></div></TableCell>
              </TableRow>)}</TableBody>
            </Table></Card> : <Card className="sim-empty-state"><FolderOpen size={28} /><b>{projects.length ? "No matching projects" : "No projects yet"}</b><span>{projects.length ? "Try another project name." : "Create a project above to start your first lab."}</span></Card>}
          </section>
  );
}
