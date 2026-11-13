import { useMemo, useState } from "react";
import { Cpu, Router, Shield, Network, Layers3, Search, PanelLeftClose } from "lucide-react";
import type { RuntimeTemplate } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { DeviceTemplateCard } from "./WorkspaceDeviceComponents";

type Category = "end-devices" | "routers" | "switches" | "security" | "all";
const categories: Array<{ id: Category; label: string; icon: typeof Cpu }> = [
  { id: "end-devices", label: "End devices", icon: Cpu },
  { id: "routers", label: "Routers", icon: Router },
  { id: "switches", label: "Switches", icon: Network },
  { id: "security", label: "Security devices", icon: Shield },
  { id: "all", label: "All devices", icon: Layers3 },
];

export function templateCategory(template: RuntimeTemplate): Category {
  const category = `${(template as RuntimeTemplate & { category?: string }).category ?? ""} ${template.template_type ?? ""} ${template.name}`.toLowerCase();
  if (/firewall|security|asa|fortigate|palo|checkpoint/.test(category)) return "security";
  if (/ethernet.?switch|switch|hub/.test(category)) return "switches";
  if (/dynamips|router/.test(category)) return "routers";
  return "end-devices";
}

type Props = { workspace: SimulatorWorkspaceProps; deviceQuery: string; setDeviceQuery: (value: string) => void; groupedTemplates: Array<{ kind: string; templates: RuntimeTemplate[] }>; expanded: boolean; onExpandedChange: (expanded: boolean) => void };
export function WorkspaceSidebar({ workspace, deviceQuery, setDeviceQuery, groupedTemplates, expanded, onExpandedChange }: Props) {
  const [active, setActive] = useState<Category | null>(null);
  const templates = useMemo(() => groupedTemplates.flatMap((group) => group.templates), [groupedTemplates]);
  const visible = templates.filter((template) => (active === "all" || templateCategory(template) === active) && (!deviceQuery.trim() || template.name.toLowerCase().includes(deviceQuery.trim().toLowerCase())));
  const activeCategory = categories.find((item) => item.id === active);
  function chooseCategory(id: Category) {
    if (expanded && active === id) {
      setActive(null);
      onExpandedChange(false);
    } else {
      setActive(id);
      onExpandedChange(true);
    }
  }
  function collapse() {
    setActive(null);
    onExpandedChange(false);
  }
  return <aside className="relative z-10 col-start-1 row-start-1 flex min-h-0 min-w-0 flex-row items-stretch overflow-visible border-r border-sidebar-border bg-sidebar text-sidebar-foreground" aria-label="Device types">
    <div className="flex w-12 shrink-0 flex-col items-center">
      <nav className="flex w-full flex-col items-center gap-1 border-b border-border p-1.5" aria-label="Device categories">
        {categories.map(({ id, label, icon: Icon }) => <Button key={id} variant={active === id ? "secondary" : "ghost"} size="icon" title={label} aria-label={label} aria-pressed={active === id} onClick={() => chooseCategory(id)}><Icon /></Button>)}
      </nav>
    </div>
    {expanded && <section className="absolute inset-y-0 left-12 z-10 flex min-h-0 w-80 min-w-0 flex-1 flex-col overflow-hidden border-r border-sidebar-border bg-sidebar text-sidebar-foreground shadow-lg animate-in slide-in-from-left-2 duration-150" aria-label={`${activeCategory?.label ?? "All devices"} templates`}>
      <header className="flex min-h-11 items-center justify-between gap-1 border-b border-sidebar-border px-3"><div className="flex min-w-0 flex-col gap-0.5"><b className="truncate text-[11px] font-semibold">{activeCategory?.label ?? "Device library"}</b><small className="text-[10px] text-muted-foreground">{visible.length} template{visible.length === 1 ? "" : "s"}</small></div><Button variant="ghost" size="icon-xs" aria-label="Collapse device sidebar" title="Collapse sidebar" onClick={collapse}><PanelLeftClose /></Button></header>
      <div className="relative flex items-center p-2"><Search className="absolute left-4 z-10 size-3.5 text-muted-foreground" /><Input className="h-[34px] pl-[30px] text-sm" value={deviceQuery} onChange={(event) => setDeviceQuery(event.target.value)} placeholder="Filter devices…" aria-label="Filter devices" /></div>
      <ScrollArea className="min-h-0 flex-1 overflow-y-auto px-2">{visible.length ? visible.map((template) => <DeviceTemplateCard key={template.template_id} template={template} onAdd={() => workspace.onAddTemplate(template.template_id)} onSave={workspace.onSaveTemplate} />) : <p className="m-2 rounded-md border border-dashed border-sidebar-border p-5 text-center text-xs text-muted-foreground">No templates in this category.</p>}</ScrollArea>
      <Button variant="outline" className="mx-2 my-2 h-[34px] shrink-0 justify-center text-xs" onClick={workspace.onOpenMarketplace}><Search />Find devices and images</Button>
    </section>}
  </aside>;
}
