import { useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent, PointerEvent } from "react";
import {
  Activity, Cable, Check, ChevronDown, Cpu, HardDrive, Link2,
  ListTree, Monitor, Network, Pause, Pencil, Play, Plus, Power, Router, Search,
  Scan, Server, Square, Terminal, Trash2, ZoomIn, ZoomOut,
} from "lucide-react";
import type { ComponentType } from "react";
import type { Drawing, Link, LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuLabel,
  ContextMenuSeparator, ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { sanitizeDrawingSvg } from "./topology";

type Selection = { kind: "node" | "link"; id: string } | null;
type LifecycleAction = "start" | "stop" | "suspend" | "reload";

export type SimulatorWorkspaceProps = {
  project: Project;
  topology: TopologySnapshot;
  templates: RuntimeTemplate[];
  runtimeInfo: RuntimeInfo | null;
  resources: ResourceMonitor | null;
  batchLifecycleRunning: boolean;
  selection: Selection;
  selectedNode: Node | null | undefined;
  selectedLink: Link | null | undefined;
  connecting: boolean;
  linkEndpoints: LinkEndpoint[];
  onBack: () => void;
  onOpenNodeDialog: () => void;
  onOpenMarketplace: () => void;
  onAddTemplate: (templateId: string, x?: number, y?: number) => void;
  onNodeLifecycle: (id: string, action: LifecycleAction) => void;
  onStartAll: () => void;
  onStopAll: () => void;
  onOpenConsole: (node: Node) => void;
  onToggleConnect: () => void;
  onCreateLink: () => void;
  onDeleteSelection: () => void;
  onNodeRename: (id: string, name: string) => void;
  onSelectNode: (id: string) => void;
  onSelectLink: (id: string) => void;
  onSelectCanvas: () => void;
  onSelectPort: (endpoint: LinkEndpoint) => void;
  onNodeContext: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
};

const scene = { width: 1440, height: 920 };

export function SimulatorWorkspace(props: SimulatorWorkspaceProps) {
  const [view, setView] = useState({ zoom: 1, centerX: 0, centerY: 0 });
  const [deviceQuery, setDeviceQuery] = useState("");
  const [rightPanel, setRightPanel] = useState("devices");
  const groupedTemplates = useMemo(() => {
    const query = deviceQuery.trim().toLowerCase();
    return ["dynamips", "qemu", "docker", "vpcs"].map((kind) => ({
      kind,
      templates: props.templates.filter((template) => template.template_type === kind && (!query || `${template.name} ${template.template_type}`.toLowerCase().includes(query))),
    }));
  }, [deviceQuery, props.templates]);
  const selectedNode = props.selectedNode;

  useEffect(() => {
    if (props.selection) setRightPanel("topology");
  }, [props.selection?.id, props.selection?.kind]);

  function fitTopology() {
    if (props.topology.nodes.length === 0) {
      setView({ zoom: 1, centerX: 0, centerY: 0 });
      return;
    }
    const xValues = props.topology.nodes.map((node) => Number(node.x ?? 0));
    const yValues = props.topology.nodes.map((node) => Number(node.y ?? 0));
    const minX = Math.min(...xValues);
    const maxX = Math.max(...xValues);
    const minY = Math.min(...yValues);
    const maxY = Math.max(...yValues);
    const zoom = Math.max(0.4, Math.min(1.8, Math.min(scene.width / (maxX - minX + 300), scene.height / (maxY - minY + 250))));
    setView({ zoom, centerX: (minX + maxX) / 2, centerY: (minY + maxY) / 2 });
  }

  return (
    <section className="sim-workspace" aria-label={`Project ${props.project.name}`}>
      <header className="sim-toolbar">
        <div className="sim-project-title">
          <Button variant="ghost" size="icon" onClick={props.onBack} aria-label="Back to projects"><ChevronDown className="rotate-90" /></Button>
          <div className="sim-project-mark"><Network size={17} /></div>
          <div className="min-w-0">
            <div className="sim-breadcrumb">Projects <span>/</span> Lab</div>
            <h1 title={props.project.name}>{props.project.name}</h1>
          </div>
          <Badge variant="outline" className="capitalize">{props.project.status ?? "opened"}</Badge>
        </div>
        <div className="sim-toolbar-actions">
          <Button variant="outline" onClick={props.onStartAll} disabled={props.batchLifecycleRunning || !props.topology.nodes.some((node) => node.status !== "started")} title="Start every stopped device"><Play />{props.batchLifecycleRunning ? "Working…" : "Start all"}</Button>
          <Button variant="outline" onClick={props.onStopAll} disabled={props.batchLifecycleRunning || !props.topology.nodes.some((node) => node.status === "started")} title="Stop every running device"><Square />Stop all</Button>
          <Separator orientation="vertical" className="h-7" />
          <Button variant={props.connecting ? "default" : "outline"} aria-pressed={props.connecting} onClick={props.onToggleConnect}><Cable />{props.connecting ? "Cancel link" : "Connect ports"}</Button>
          {props.connecting && <Button onClick={props.onCreateLink} disabled={props.linkEndpoints.length < 2}><Link2 />Create link{props.linkEndpoints.length > 0 ? ` · ${props.linkEndpoints.length}` : ""}</Button>}
          <Button variant="outline" onClick={props.onOpenMarketplace}><Search />Image catalog</Button>
        </div>
      </header>

      {props.connecting && <div className="sim-link-hint"><Cable size={15} /><span>Select ports on the canvas to build an Ethernet segment.</span><Badge variant="secondary">{props.linkEndpoints.length} selected</Badge></div>}

      <div className="sim-workspace-grid">
        <Card className="sim-canvas-card">
          <div className="sim-canvas-bar">
            <div className="flex min-w-0 items-center gap-2 text-sm">
              <span className="sim-live-dot" />
              <span className="font-medium">Topology map</span>
              <Separator orientation="vertical" className="mx-1 h-4" />
              <span className="text-muted-foreground">{props.topology.nodes.length} devices</span>
              <span className="text-muted-foreground">·</span>
              <span className="text-muted-foreground">{props.topology.links.length} links</span>
            </div>
            <div className="flex items-center gap-1">
              <Button variant="ghost" size="icon-sm" aria-label="Zoom out" onClick={() => setView((current) => ({ ...current, zoom: Math.max(0.4, Number((current.zoom - 0.1).toFixed(1))) }))}><ZoomOut /></Button>
              <span className="w-12 text-center font-mono text-xs text-muted-foreground">{Math.round(view.zoom * 100)}%</span>
              <Button variant="ghost" size="icon-sm" aria-label="Zoom in" onClick={() => setView((current) => ({ ...current, zoom: Math.min(1.8, Number((current.zoom + 0.1).toFixed(1))) }))}><ZoomIn /></Button>
              <Button variant="ghost" size="icon-sm" aria-label="Fit topology" title="Fit all devices" onClick={fitTopology}><Scan /></Button>
            </div>
          </div>
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div className={`sim-canvas ${props.connecting ? "is-connecting" : ""}`}>
                <TopologyCanvas
                  nodes={props.topology.nodes}
                  links={props.topology.links}
                  drawings={props.topology.drawings}
                  zoom={view.zoom}
                  centerX={view.centerX}
                  centerY={view.centerY}
                  onCenterChange={(centerX, centerY) => setView((current) => ({ ...current, centerX, centerY }))}
                  selection={props.selection}
                  connecting={props.connecting}
                  linkEndpoints={props.linkEndpoints}
                  onNodeClick={props.onSelectNode}
                  onLinkClick={props.onSelectLink}
                  onCanvasClick={props.onSelectCanvas}
                  onPortClick={props.onSelectPort}
                  onNodeContext={props.onNodeContext}
                  onNodeMove={props.onNodeMove}
                  onNodeMoveEnd={props.onNodeMoveEnd}
                  onDropTemplate={(templateId, x, y) => props.onAddTemplate(templateId, x, y)}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent className="w-56">
              <ContextMenuLabel>{selectedNode?.name ?? (props.selectedLink ? "Ethernet link" : "Topology")}</ContextMenuLabel>
              <ContextMenuSeparator />
              {selectedNode ? <>
                <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, selectedNode.status === "started" ? "stop" : "start")}>
                  {selectedNode.status === "started" ? <Power /> : <Play />}{selectedNode.status === "started" ? "Stop device" : "Start device"}
                </ContextMenuItem>
                {selectedNode.status === "started" && <ContextMenuItem onSelect={() => props.onOpenConsole(selectedNode)}><Terminal />Open console</ContextMenuItem>}
                <ContextMenuItem onSelect={() => { document.querySelector<HTMLButtonElement>("[data-tab=topology]")?.click(); window.setTimeout(() => document.querySelector<HTMLInputElement>("[data-rename-node]")?.focus(), 0); }}><Pencil />Edit device name</ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem variant="destructive" onSelect={props.onDeleteSelection}><Trash2 />Delete device</ContextMenuItem>
              </> : props.selectedLink ? <ContextMenuItem variant="destructive" onSelect={props.onDeleteSelection}><Trash2 />Delete link</ContextMenuItem> : <ContextMenuItem onSelect={props.onOpenNodeDialog}><Plus />Import or configure device</ContextMenuItem>}
            </ContextMenuContent>
          </ContextMenu>
          <footer className="sim-canvas-footer">
            <span>Drag devices from the right panel</span><span>Drag a device to move it</span><span>Right-click a device for actions</span>
            {props.connecting && <span className="text-primary">Click interface ports to cable devices</span>}
          </footer>
        </Card>

        <aside className="sim-device-sidebar">
          <Tabs value={rightPanel} onValueChange={setRightPanel} className="flex h-full flex-col">
            <div className="sim-sidebar-heading">
              <TabsList className="grid w-full grid-cols-2">
                <TabsTrigger value="devices" data-tab="devices"><Server />Devices</TabsTrigger>
                <TabsTrigger value="topology" data-tab="topology"><ListTree />Topology</TabsTrigger>
              </TabsList>
            </div>
            <TabsContent value="devices" className="sim-sidebar-content">
              <div className="sim-sidebar-section-title"><div><h2>Device library</h2><p>Drag a device onto the map.</p></div><Button variant="ghost" size="icon-sm" onClick={props.onOpenNodeDialog} aria-label="Add node or image" title="Add node or image"><Plus /></Button></div>
              <label className="relative block px-3 pb-3">
                <Search className="pointer-events-none absolute left-6 top-2.5 size-4 text-muted-foreground" />
                <Input value={deviceQuery} onChange={(event) => setDeviceQuery(event.target.value)} placeholder="Filter devices…" className="pl-9" aria-label="Filter devices" />
              </label>
              <ScrollArea className="sim-palette-scroll">
                <div className="space-y-2 px-3 pb-4">
                  {groupedTemplates.map(({ kind, templates }) => <Collapsible key={kind} defaultOpen={templates.length > 0}>
                    <CollapsibleTrigger className="sim-group-trigger"><span>{kindLabel(kind)}</span><Badge variant="secondary">{templates.length}</Badge><ChevronDown className="ml-auto size-4 text-muted-foreground" /></CollapsibleTrigger>
                    <CollapsibleContent className="space-y-1 pt-1">
                      {templates.map((template) => <DeviceTemplateCard key={template.template_id} template={template} onAdd={() => props.onAddTemplate(template.template_id)} />)}
                      {templates.length === 0 && <p className="px-2 py-2 text-xs text-muted-foreground">No {kindLabel(kind).toLowerCase()} templates are installed.</p>}
                    </CollapsibleContent>
                  </Collapsible>)}
                  {props.templates.length === 0 && <Card className="p-3 text-xs leading-relaxed text-muted-foreground">No device templates are available yet. Import a Cisco IOS or QEMU image, pull a Docker image, or install VPCS. Images are stored in the global library.</Card>}
                </div>
              </ScrollArea>
              <div className="sim-sidebar-bottom-actions"><Button variant="outline" className="w-full" onClick={props.onOpenMarketplace}><Search />Find devices and images</Button></div>
            </TabsContent>

            <TabsContent value="topology" className="sim-sidebar-content">
              <div className="sim-sidebar-section-title"><div><h2>Topology summary</h2><p>Device state and interfaces.</p></div><Badge variant="outline">{props.topology.nodes.length}</Badge></div>
              <ScrollArea className="sim-topology-list-scroll">
                {props.topology.nodes.length ? <div className="px-3 pb-3">
                  <div className="overflow-hidden rounded-md border">
                    <Table className="sim-node-table">
                      <TableHeader><TableRow><TableHead>Device</TableHead><TableHead className="w-24">State</TableHead></TableRow></TableHeader>
                      <TableBody>{props.topology.nodes.map((node) => <TableRow key={node.node_id} data-state={props.selection?.kind === "node" && props.selection.id === node.node_id ? "selected" : undefined} className="cursor-pointer" onClick={() => props.onSelectNode(node.node_id)}>
                        <TableCell><div className="flex min-w-0 items-center gap-2"><span className={`sim-state-dot ${node.status === "started" ? "running" : ""}`} /><span className="truncate">{node.name}</span></div></TableCell>
                        <TableCell><Badge variant={node.status === "started" ? "default" : "secondary"} className="capitalize">{node.status ?? "stopped"}</Badge></TableCell>
                      </TableRow>)}</TableBody>
                    </Table>
                  </div>
                </div> : <p className="px-5 py-8 text-center text-sm text-muted-foreground">No devices on this topology.</p>}
                {selectedNode && <NodeInspector key={selectedNode.node_id}
                  node={selectedNode}
                  links={props.topology.links}
                  nodes={props.topology.nodes}
                  onLifecycle={props.onNodeLifecycle}
                  onOpenConsole={props.onOpenConsole}
                  onRename={props.onNodeRename}
                  onDelete={props.onDeleteSelection}
                />}
                {props.selectedLink && <Card className="mx-3 mb-3 p-3">
                  <div className="flex items-center justify-between"><div><p className="text-sm font-medium">Ethernet segment</p><p className="mt-1 text-xs text-muted-foreground">{props.selectedLink.nodes.length} connected interfaces</p></div><Button variant="ghost" size="icon-sm" onClick={props.onDeleteSelection} aria-label="Delete selected link"><Trash2 /></Button></div>
                  <Separator className="my-3" />
                  <div className="space-y-2">{props.selectedLink.nodes.map((endpoint) => { const node = props.topology.nodes.find((item) => item.node_id === endpoint.node_id); return <div key={`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`} className="flex justify-between gap-3 text-xs"><span className="truncate">{node?.name ?? endpoint.node_id}</span><span className="font-mono text-muted-foreground">{portLabel(node, endpoint)}</span></div>; })}</div>
                </Card>}
              </ScrollArea>
              <div className="sim-topology-summary"><div><span>Links</span><b>{props.topology.links.length}</b></div><div><span>Drawing objects</span><b>{props.topology.drawings.length}</b></div><div><span>Controller events</span><b>#{props.topology.event_cursor}</b></div></div>
            </TabsContent>
          </Tabs>
        </aside>
      </div>
    </section>
  );
}

function DeviceTemplateCard({ template, onAdd }: { template: RuntimeTemplate; onAdd: () => void }) {
  const Icon = iconForType(template.template_type);
  return <div className="sim-device-template" draggable onDragStart={(event) => {
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-network-simulator-template", template.template_id);
    event.dataTransfer.setData("text/plain", template.name);
  }}>
    <span className="sim-template-icon"><Icon size={17} /></span>
    <span className="sim-template-copy"><b title={template.name}>{template.name}</b><small>{templateDescription(template)}</small></span>
    <Button variant="ghost" size="icon-xs" onClick={onAdd} aria-label={`Add ${template.name}`} title="Add to topology"><Plus /></Button>
  </div>;
}

function NodeInspector({ node, links, nodes, onLifecycle, onOpenConsole, onRename, onDelete }: {
  node: Node;
  links: Link[];
  nodes: Node[];
  onLifecycle: (id: string, action: LifecycleAction) => void;
  onOpenConsole: (node: Node) => void;
  onRename: (id: string, name: string) => void;
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
      {node.status === "started" ? <Button variant="outline" size="sm" onClick={() => onLifecycle(node.node_id, "stop")}><Power />Stop</Button> : <Button size="sm" disabled={Boolean(node.runtime_missing)} onClick={() => onLifecycle(node.node_id, "start")}><Play />Start</Button>}
      {canOpenConsole && <Button variant="outline" size="sm" onClick={() => onOpenConsole(node)}><Terminal />Open console</Button>}
      <Button variant="outline" size="icon-sm" disabled={node.status !== "started" || node.node_type === "vpcs"} onClick={() => onLifecycle(node.node_id, "suspend")} aria-label="Suspend device" title="Suspend"><Pause /></Button>
      <Button variant="outline" size="icon-sm" disabled={node.status !== "started"} onClick={() => onLifecycle(node.node_id, "reload")} aria-label="Restart device" title="Restart"><Activity /></Button>
    </div>
    <Separator className="my-3" />
    <div className="space-y-2">
      <div className="sim-detail-row"><span>Interfaces</span><b>{portCount(node)}</b></div>
      <div className="sim-detail-row"><span>Node ID</span><code>{node.node_id.slice(0, 8)}</code></div>
      {linkedPorts.length ? <div className="space-y-2 pt-1"><p className="text-xs font-medium text-muted-foreground">Connected interfaces</p>{linkedPorts.map(({ endpoint, peers }) => <div key={`${endpoint.adapter_number}:${endpoint.port_number}`} className="flex items-center justify-between gap-2 text-xs"><span className="font-mono">{portLabel(node, endpoint)}</span><span className="truncate text-muted-foreground">{peers.join(", ")}</span></div>)}</div> : <p className="text-xs text-muted-foreground">No connected interfaces.</p>}
    </div>
  </Card>;
}

type CanvasProps = {
  nodes: Node[];
  links: Link[];
  drawings: Drawing[];
  zoom: number;
  centerX: number;
  centerY: number;
  onCenterChange: (x: number, y: number) => void;
  selection: Selection;
  connecting: boolean;
  linkEndpoints: LinkEndpoint[];
  onNodeClick: (id: string) => void;
  onLinkClick: (id: string) => void;
  onCanvasClick: () => void;
  onPortClick: (endpoint: LinkEndpoint) => void;
  onNodeContext: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
  onDropTemplate: (templateId: string, x: number, y: number) => void;
};

type DragState = { nodeId: string; pointerId: number };
type PanState = { pointerId: number; startX: number; startY: number; centerX: number; centerY: number };

function TopologyCanvas(props: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const panRef = useRef<PanState | null>(null);
  const spaceDownRef = useRef(false);
  const panMovedRef = useRef(false);
  const movedRef = useRef(false);
  const pendingPositionRef = useRef<{ nodeId: string; x: number; y: number } | null>(null);
  const animationFrameRef = useRef<number | null>(null);
  const [dropActive, setDropActive] = useState(false);
  const view = { width: scene.width / props.zoom, height: scene.height / props.zoom, x: props.centerX - scene.width / props.zoom / 2, y: props.centerY - scene.height / props.zoom / 2 };
  const nodeMap = useMemo(() => new Map(props.nodes.map((node) => [node.node_id, node])), [props.nodes]);
  const renderedDrawings = useMemo(() => props.drawings.flatMap((drawing) => {
    const markup = typeof drawing.svg === "string" ? sanitizeDrawingSvg(drawing.svg) : "";
    return markup ? [{ drawing, markup }] : [];
  }), [props.drawings]);

  useEffect(() => () => {
    if (animationFrameRef.current !== null) cancelAnimationFrame(animationFrameRef.current);
  }, []);

  const scenePoint = (clientX: number, clientY: number, rect: DOMRect) => {
    const scale = Math.min(rect.width / view.width, rect.height / view.height);
    const offsetX = (rect.width - view.width * scale) / 2;
    const offsetY = (rect.height - view.height * scale) / 2;
    return { x: Math.round((view.x + (clientX - rect.left - offsetX) / scale) / 10) * 10, y: Math.round((view.y + (clientY - rect.top - offsetY) / scale) / 10) * 10 };
  };

  function startMove(event: PointerEvent<SVGGElement>, node: Node) {
    if (props.connecting || event.button !== 0 || (event.target as Element).closest("[data-interface-port]")) return;
    event.stopPropagation();
    dragRef.current = { nodeId: node.node_id, pointerId: event.pointerId };
    movedRef.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startPan(event: PointerEvent<SVGSVGElement>) {
    const isBackground = event.target === event.currentTarget || (event.target as Element).tagName === "rect";
    const panButton = event.button === 1 || (event.button === 0 && spaceDownRef.current);
    if (!isBackground || !panButton) return;
    event.preventDefault();
    panRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, centerX: props.centerX, centerY: props.centerY };
    panMovedRef.current = false;
    svgRef.current?.setPointerCapture(event.pointerId);
  }

  function moveNode(event: PointerEvent<SVGSVGElement>) {
    const pan = panRef.current;
    if (pan && pan.pointerId === event.pointerId && svgRef.current) {
      const rect = svgRef.current.getBoundingClientRect();
      const scale = Math.min(rect.width / view.width, rect.height / view.height);
      const dx = (event.clientX - pan.startX) / scale;
      const dy = (event.clientY - pan.startY) / scale;
      if (Math.abs(dx) + Math.abs(dy) > 2) panMovedRef.current = true;
      props.onCenterChange(pan.centerX - dx, pan.centerY - dy);
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId || !svgRef.current) return;
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    movedRef.current = true;
    pendingPositionRef.current = { nodeId: drag.nodeId, x: point.x, y: point.y };
    if (animationFrameRef.current === null) {
      animationFrameRef.current = requestAnimationFrame(() => {
        animationFrameRef.current = null;
        const pending = pendingPositionRef.current;
        if (pending) props.onNodeMove(pending.nodeId, pending.x, pending.y);
      });
    }
  }

  function endMove(event: PointerEvent<SVGSVGElement>) {
    if (panRef.current?.pointerId === event.pointerId) {
      panRef.current = null;
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (movedRef.current && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      if (animationFrameRef.current !== null) cancelAnimationFrame(animationFrameRef.current);
      animationFrameRef.current = null;
      pendingPositionRef.current = null;
      props.onNodeMove(drag.nodeId, point.x, point.y);
      props.onNodeMoveEnd(drag.nodeId, point.x, point.y);
    }
  }

  function dropTemplate(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDropActive(false);
    const templateId = event.dataTransfer.getData("application/x-network-simulator-template");
    if (!templateId || !svgRef.current) return;
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    props.onDropTemplate(templateId, point.x, point.y);
  }

  return <div className={`sim-canvas-surface ${dropActive ? "is-drop-active" : ""}`} onDragOver={(event) => { if (event.dataTransfer.types.includes("application/x-network-simulator-template")) { event.preventDefault(); event.dataTransfer.dropEffect = "copy"; setDropActive(true); } }} onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as globalThis.Node | null)) setDropActive(false); }} onDrop={dropTemplate}>
    <svg ref={svgRef} viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} role="group" aria-label="Network topology canvas" tabIndex={0} onPointerDown={startPan} onPointerMove={moveNode} onPointerUp={endMove} onPointerCancel={endMove} onKeyDown={(event) => { if (event.code === "Space") { event.preventDefault(); spaceDownRef.current = true; } }} onKeyUp={(event) => { if (event.code === "Space") spaceDownRef.current = false; }} onBlur={() => { spaceDownRef.current = false; }} onClick={(event) => { if (panMovedRef.current) { panMovedRef.current = false; return; } if (event.target === event.currentTarget || (event.target as Element).tagName === "rect") props.onCanvasClick(); }} onContextMenu={(event) => { if ((event.target as Element).tagName === "rect") props.onCanvasClick(); }}>
      <defs><pattern id="sim-grid" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="currentColor" /></pattern></defs>
      <rect className="sim-grid-fill" x={view.x} y={view.y} width={view.width} height={view.height} fill="url(#sim-grid)" />
      {renderedDrawings.map(({ drawing, markup }) => <g key={drawing.drawing_id} transform={`translate(${Number(drawing.x ?? 0)},${Number(drawing.y ?? 0)}) rotate(${Number(drawing.rotation ?? 0)})`} style={{ pointerEvents: "none" }} dangerouslySetInnerHTML={{ __html: markup }} />)}
      {props.links.map((link) => {
        const endpoints = (link.nodes ?? []).map((endpoint) => ({ endpoint, node: nodeMap.get(endpoint.node_id) })).filter((item): item is { endpoint: LinkEndpoint; node: Node } => Boolean(item.node));
        const source = endpoints[0];
        if (!source || endpoints.length < 2) return null;
        const selected = props.selection?.kind === "link" && props.selection.id === link.link_id;
        const sourcePoint = portPoint(source.node, source.endpoint);
        return <g key={link.link_id} className={`sim-link ${selected ? "is-selected" : ""}`} onClick={(event) => { event.stopPropagation(); props.onLinkClick(link.link_id); }} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") props.onLinkClick(link.link_id); }} role="button" aria-label={`Ethernet segment connecting ${endpoints.map(({ node }) => node.name).join(", ")}`} tabIndex={0}>
          {endpoints.slice(1).map(({ endpoint, node }) => { const point = portPoint(node, endpoint); return <g key={`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`}><line x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} /><line className="sim-link-hit" x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} /></g>; })}
        </g>;
      })}
      {props.nodes.map((node) => {
        const x = Number(node.x ?? 0);
        const y = Number(node.y ?? 0);
        const selected = props.selection?.kind === "node" && props.selection.id === node.node_id;
        const Icon = iconForType(node.node_type ?? "");
        const endpoints = nodePorts(node);
        return <g key={node.node_id} transform={`translate(${x},${y})`} className={`sim-node ${selected ? "is-selected" : ""} ${props.connecting ? "is-connecting" : ""}`} onPointerDown={(event) => startMove(event, node)} onClick={(event) => { event.stopPropagation(); if (!movedRef.current) props.onNodeClick(node.node_id); movedRef.current = false; }} onContextMenu={() => props.onNodeContext(node.node_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onNodeClick(node.node_id); } }} role="button" aria-label={`Node ${node.name}`} tabIndex={0}>
          <rect className="sim-node-card" x="-56" y="-36" width="112" height="66" rx="9" />
          <Icon className="sim-node-icon" x={-10} y={-24} width={20} height={20} strokeWidth={1.7} />
          <text className="sim-node-type" y="5">{kindLabel(node.node_type ?? "device")}</text>
          <circle className={`sim-node-status ${node.status === "started" ? "running" : "stopped"}`} cx="46" cy="-26" r="4" />
          <text className="sim-node-name" y="52">{node.name}</text>
          {endpoints.map((endpoint, index) => {
            const point = localPortPoint(index, endpoints.length);
            const chosen = props.linkEndpoints.some((item) => sameEndpoint(item, endpoint));
            const connected = props.links.some((link) => link.nodes?.some((item) => sameEndpoint(item, endpoint)));
            return <g key={`${endpoint.adapter_number}:${endpoint.port_number}`} data-interface-port="true" className={`sim-interface ${chosen ? "is-chosen" : ""} ${connected ? "is-connected" : ""}`} onPointerDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); props.onPortClick(endpoint); }} role="button" aria-label={`${node.name} ${portLabel(node, endpoint)}`} tabIndex={0}>
              <circle cx={point.x} cy={point.y} r="5.5" />
              <text x={point.x} y={point.y + 15}>{endpointLabel(node, endpoint)}</text>
            </g>;
          })}
        </g>;
        })}
      {props.nodes.length === 0 && <g className="sim-canvas-empty"><Network x={-16} y={-30} width={32} height={32} /><text y="54">Your canvas is ready</text><text className="sim-empty-subtitle" y="77">Drop a device here or add one from the library on the right.</text></g>}
    </svg>
    <div className="sim-coordinate-readout">{props.nodes.length} devices · {props.links.length} Ethernet segments</div>
  </div>;
}

function sameEndpoint(left: LinkEndpoint, right: LinkEndpoint) {
  return left.node_id === right.node_id && left.adapter_number === right.adapter_number && left.port_number === right.port_number;
}

function portCount(node: Node): number {
  const value = Number(node.adapters ?? node.adapter_count ?? node.ports_count ?? (node.node_type === "vpcs" || node.node_type === "dynamips" ? 1 : 4));
  return Number.isFinite(value) ? Math.max(1, Math.min(16, value)) : 4;
}

function nodePorts(node: Node): LinkEndpoint[] {
  return Array.from({ length: portCount(node) }, (_, index) => node.node_type === "qemu"
    ? { node_id: node.node_id, adapter_number: index, port_number: 0 }
    : { node_id: node.node_id, adapter_number: 0, port_number: index });
}

function portIndex(node: Node, endpoint: LinkEndpoint) {
  return node.node_type === "qemu" ? endpoint.adapter_number : endpoint.port_number;
}

function localPortPoint(index: number, total: number) {
  const columns = Math.min(8, total);
  const row = Math.floor(index / 8);
  const column = index % 8;
  return { x: (column - (Math.min(columns, total - row * 8) - 1) / 2) * 13, y: 27 + row * 10 };
}

function portPoint(node: Node, endpoint: LinkEndpoint) {
  const point = localPortPoint(portIndex(node, endpoint), portCount(node));
  return { x: Number(node.x ?? 0) + point.x, y: Number(node.y ?? 0) + point.y };
}

function portLabel(node: Node | undefined, endpoint: LinkEndpoint) {
  return node?.node_type === "qemu" ? `Ethernet${endpoint.adapter_number}` : node?.node_type === "dynamips" ? `Ethernet${endpoint.port_number}` : `eth${endpoint.port_number}`;
}

function endpointLabel(node: Node, endpoint: LinkEndpoint) {
  const index = portIndex(node, endpoint);
  return node.node_type === "qemu" ? `e${index}` : node.node_type === "dynamips" ? `e${index}` : `${index}`;
}

function iconForType(type: string): ComponentType<{ size?: number; className?: string; x?: number; y?: number; width?: number; height?: number; strokeWidth?: number }> {
  if (type === "qemu") return Monitor;
  if (type === "dynamips") return Router;
  if (type === "docker") return Server;
  if (type === "vpcs") return Cpu;
  return HardDrive;
}

function kindLabel(kind: string) {
  return ({ dynamips: "Cisco IOS", qemu: "QEMU / KVM", docker: "Docker", vpcs: "VPCS", device: "Device" } as Record<string, string>)[kind] ?? kind;
}

function templateDescription(template: RuntimeTemplate) {
  if (template.template_type === "qemu") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · ${template.memory} MB RAM`;
  if (template.template_type === "docker") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · Docker image`;
  if (template.template_type === "dynamips") return `${template.platform} · Cisco IOS`;
  return "Lightweight virtual PC";
}

function imageName(node: Node) {
  const image = typeof node.image === "string" ? node.image : typeof node.hda_disk_image === "string" ? node.hda_disk_image : "";
  return image.split("/").at(-1) || kindLabel(node.node_type ?? "device");
}
