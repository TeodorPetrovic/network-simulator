import { useState } from "react";
import { Activity, Check, Pause, Pencil, Play, Plus, Power, Settings2, Terminal, Trash2 } from "lucide-react";
import type { Link, Node, RuntimeTemplate } from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { canApplyLifecycle } from "./lifecycle";
import type { LifecycleAction } from "./lifecycle";
import { TemplateSettingsDialog } from "./TemplateSettingsDialog";
import { iconForType, imageName, portCount, portLabel, templateDescription } from "./map/TopologyCanvas";

export function DeviceTemplateCard({ template, onAdd, onSave, onDragEnd }: { template: RuntimeTemplate; onAdd: () => void; onSave: (id: string, patch: Record<string, unknown>) => Promise<void>; onDragEnd?: () => void }) {
  const Icon = iconForType(template.template_type);
  const [settingsOpen, setSettingsOpen] = useState(false);
  return <>
  <div className="sim-device-template" draggable onDragStart={(event) => {
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-network-simulator-template", template.template_id);
    event.dataTransfer.setData("text/plain", template.name);
  }} onDragEnd={onDragEnd}>
    <span className="sim-template-icon" draggable={false} style={{ pointerEvents: "none" }}><Icon size={17} /></span>
    <span className="sim-template-copy"><b title={template.name}>{template.name}</b><small>{templateDescription(template)}</small></span>
    <Button variant="ghost" size="icon-xs" onClick={() => setSettingsOpen(true)} aria-label={`Configure ${template.name} template`} title="Configure template"><Settings2 /></Button>
    <Button variant="ghost" size="icon-xs" onClick={onAdd} aria-label={`Add ${template.name}`} title="Add to topology"><Plus /></Button>
  </div>
    <TemplateSettingsDialog template={template} open={settingsOpen} onOpenChange={setSettingsOpen} onSave={onSave} />
  </>;
}

export function NodeInspector({ node, links, nodes, onLifecycle, onOpenConsole, onRename, onConfigure, onDelete }: {
  node: Node;
  links: Link[];
  nodes: Node[];
  onLifecycle: (id: string, action: LifecycleAction) => void;
  onOpenConsole: (node: Node, auxiliary?: boolean) => void;
  onRename: (id: string, name: string) => void;
  onConfigure: () => void;
  onDelete: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(node.name);
  const [saving, setSaving] = useState(false);
  const linkedPorts = links.flatMap((link) => link.nodes.filter((endpoint) => endpoint.node_id === node.node_id).map((endpoint) => ({
    endpoint,
    peers: link.nodes.filter((peer) => peer.node_id !== node.node_id).map((peer) => nodes.find((item) => item.node_id === peer.node_id)?.name ?? "Unknown device"),
  })));
  const canOpenConsole = ["docker", "qemu", "vpcs", "dynamips"].includes(node.node_type ?? "") && node.status === "started";

  async function saveName() {
    const trimmed = name.trim();
    if (!trimmed || trimmed === node.name) { setEditing(false); setName(node.name); return; }
    setSaving(true);
    try { await onRename(node.node_id, trimmed); setEditing(false); }
    catch { /* The parent reports the controller error in the application status area. */ }
    finally { setSaving(false); }
  }

  return <Card className="mx-3 mb-3 p-3">
    <div className="flex items-start justify-between gap-2">
      <div className="min-w-0 flex-1">
        <div className="mb-2 flex items-center gap-2"><Badge variant="outline" className="capitalize">{node.node_type ?? "device"}</Badge><Badge variant={node.status === "started" ? "default" : "secondary"} className="capitalize">{node.status ?? "stopped"}</Badge></div>
        {editing ? <div className="flex gap-1"><Input data-rename-node autoFocus value={name} onChange={(event) => setName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void saveName(); if (event.key === "Escape") { setEditing(false); setName(node.name); } }} aria-label="Device name" /><Button size="icon" disabled={saving || !name.trim()} onClick={() => void saveName()} aria-label="Save device name"><Check /></Button></div> : <div className="flex items-center gap-1"><h3 className="truncate text-sm font-semibold" title={node.name}>{node.name}</h3><Button variant="ghost" size="icon-xs" onClick={() => { setName(node.name); setEditing(true); }} aria-label="Rename device"><Pencil /></Button></div>}
        <p className="mt-1 truncate text-xs text-muted-foreground">{imageName(node)}</p>
      </div>
      <Button variant="ghost" size="icon-xs" onClick={onDelete} aria-label="Delete device"><Trash2 /></Button>
    </div>
    <div className="mt-3 flex flex-wrap gap-2">
      {["qemu", "docker", "vpcs", "dynamips"].includes(node.node_type ?? "") && <Button variant="outline" size="sm" onClick={onConfigure}><Settings2 />Configure</Button>}
      {canApplyLifecycle(node, "start") && <Button size="sm" disabled={Boolean(node.runtime_missing)} onClick={() => onLifecycle(node.node_id, "start")}><Play />Start</Button>}
      {canApplyLifecycle(node, "stop") && <Button variant="outline" size="sm" onClick={() => onLifecycle(node.node_id, "stop")}><Power />Stop</Button>}
      {canOpenConsole && <Button variant="outline" size="sm" onClick={() => onOpenConsole(node)}><Terminal />Open console</Button>}
      {canApplyLifecycle(node, "suspend") && <Button variant="outline" size="icon-sm" onClick={() => onLifecycle(node.node_id, "suspend")} aria-label="Suspend device" title="Suspend"><Pause /></Button>}
      {canApplyLifecycle(node, "reload") && <Button variant="outline" size="icon-sm" onClick={() => onLifecycle(node.node_id, "reload")} aria-label="Restart device" title="Restart"><Activity /></Button>}
    </div>
    <Separator className="my-3" />
    <div className="space-y-2">
      <div className="sim-detail-row"><span>Interfaces</span><b>{portCount(node)}</b></div>
      <div className="sim-detail-row"><span>Node ID</span><code>{node.node_id.slice(0, 8)}</code></div>
      {linkedPorts.length ? <div className="space-y-2 pt-1"><p className="text-xs font-medium text-muted-foreground">Connected interfaces</p>{linkedPorts.map(({ endpoint, peers }) => <div key={`${endpoint.adapter_number}:${endpoint.port_number}`} className="flex items-center justify-between gap-2 text-xs"><span className="font-mono">{portLabel(node, endpoint)}</span><span className="truncate text-muted-foreground">{peers.join(", ")}</span></div>)}</div> : <p className="text-xs text-muted-foreground">No connected interfaces.</p>}
    </div>
  </Card>;
}
