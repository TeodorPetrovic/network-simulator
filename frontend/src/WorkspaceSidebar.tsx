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
  return <aside className={`sim-device-rail${expanded ? " is-expanded" : ""}`} aria-label="Device types">
    <div className="sim-device-rail-strip">
      <nav className="sim-device-rail-buttons" aria-label="Device categories">
        {categories.map(({ id, label, icon: Icon }) => <Button key={id} variant={active === id ? "secondary" : "ghost"} size="icon" title={label} aria-label={label} aria-pressed={active === id} onClick={() => chooseCategory(id)}><Icon /></Button>)}
      </nav>
    </div>
    {expanded && <section className="sim-device-panel" aria-label={`${activeCategory?.label ?? "All devices"} templates`}>
      <header><div><b>{activeCategory?.label ?? "Device library"}</b><small>{visible.length} template{visible.length === 1 ? "" : "s"}</small></div><Button variant="ghost" size="icon-xs" aria-label="Collapse device sidebar" title="Collapse sidebar" onClick={collapse}><PanelLeftClose /></Button></header>
      <label className="sim-device-search"><Search /><Input value={deviceQuery} onChange={(event) => setDeviceQuery(event.target.value)} placeholder="Filter devices…" aria-label="Filter devices" /></label>
      <ScrollArea className="sim-device-panel-list">{visible.length ? visible.map((template) => <DeviceTemplateCard key={template.template_id} template={template} onAdd={() => workspace.onAddTemplate(template.template_id)} onSave={workspace.onSaveTemplate} />) : <p className="sim-device-empty">No templates in this category.</p>}</ScrollArea>
      <Button variant="outline" className="sim-device-catalog" onClick={workspace.onOpenMarketplace}><Search />Find devices and images</Button>
    </section>}
  </aside>;
}
