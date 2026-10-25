import { useMemo, useState } from "react";
import { Cpu, Link2, Router, Shield, Network, Layers3, Search, X } from "lucide-react";
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

type Props = { workspace: SimulatorWorkspaceProps; deviceQuery: string; setDeviceQuery: (value: string) => void; groupedTemplates: Array<{ kind: string; templates: RuntimeTemplate[] }> };
export function WorkspaceSidebar({ workspace, deviceQuery, setDeviceQuery, groupedTemplates }: Props) {
  const [active, setActive] = useState<Category | null>(null);
  const [linkMode, setLinkMode] = useState(false);
  const templates = useMemo(() => groupedTemplates.flatMap((group) => group.templates), [groupedTemplates]);
  const visible = templates.filter((template) => (active === "all" || templateCategory(template) === active) && (!deviceQuery.trim() || template.name.toLowerCase().includes(deviceQuery.trim().toLowerCase())));
  return <aside className="sim-device-rail" aria-label="Device types">
    <nav className="sim-device-rail-buttons" aria-label="Device categories">
      {categories.map(({ id, label, icon: Icon }) => <Button key={id} variant={active === id ? "secondary" : "ghost"} size="icon" title={label} aria-label={label} aria-pressed={active === id} onClick={() => setActive((value) => value === id ? null : id)}><Icon /></Button>)}
    </nav>
    <div className="sim-device-rail-bottom"><Button variant={linkMode ? "default" : "ghost"} size="icon" title="Add a link" aria-label="Add a link" aria-pressed={linkMode} onClick={() => { setLinkMode((value) => !value); workspace.onToggleConnect(); }}><Link2 /></Button></div>
    {active && <section className="sim-device-flyout" aria-label={`${categories.find((item) => item.id === active)?.label} templates`}>
      <header><b>{categories.find((item) => item.id === active)?.label}</b><Button variant="ghost" size="icon-xs" aria-label="Close device list" onClick={() => setActive(null)}><X /></Button></header>
      <label className="sim-device-search"><Search /><Input value={deviceQuery} onChange={(event) => setDeviceQuery(event.target.value)} placeholder="Filter devices…" aria-label="Filter devices" /></label>
      <ScrollArea className="sim-device-flyout-list">{visible.length ? visible.map((template) => <DeviceTemplateCard key={template.template_id} template={template} onAdd={() => workspace.onAddTemplate(template.template_id)} onSave={workspace.onSaveTemplate} />) : <p className="sim-device-empty">No templates in this category.</p>}</ScrollArea>
      <Button variant="outline" className="sim-device-catalog" onClick={workspace.onOpenMarketplace}><Search />Find devices and images</Button>
    </section>}
  </aside>;
}
