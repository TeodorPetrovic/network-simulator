import { useEffect, useMemo, useState } from "react";
import {
  Activity, Cable, ChevronDown, HelpCircle, Image, Link2, ListTree, Network, Pause, Pencil,
  Play, Plus, Power, Search, Scan, Server, Settings2, Square, Terminal, Trash2, ZoomIn,
  ZoomOut, Lock, Unlock, MoreHorizontal, Camera, RectangleHorizontal, Circle, Minus, Type,
  Filter, Layers,
} from "lucide-react";
import type { Link, LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { api } from "./api";
import { canApplyLifecycle, nodesForLifecycle } from "./lifecycle";
import type { LifecycleAction } from "./lifecycle";
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
import { ResourceMonitorPopover } from "./ResourceMonitorPopover";
import { NodeSettingsDialog } from "./NodeSettingsDialog";
import { DeviceTemplateCard, NodeInspector } from "./WorkspaceDeviceComponents";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { DrawingEditorDialog, HelpDialog, InfoDialog, InterfacePickerDialog, LinkPropertiesDialog, LogConsoleDialog, ScreenshotDialog, SnapshotsDialog, SymbolPickerDialog } from "./map/MapDialogs";
import type { ScreenshotRequest } from "./map/MapDialogs";
import { drawingSvg, reorderDrawings, serializeSvgScreenshot, svgScreenshotPng } from "./map/helpers";
import type { DrawingTool } from "./map/helpers";
import { TopologyCanvas, scene, maxZoom, messageFor, portLabel, safeFileName, sameEndpoint, valuesFromDrawing, deviceGroupLabel, nodeHasAuxConsole } from "./map/TopologyCanvas";
import { pngScreenshotAvailable, spreadOverlappingNodes } from "./workspaceUtils";
import type { Selection } from "./map/TopologyCanvas";

type DrawingValues = { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string };

export type SimulatorWorkspaceProps = {
  project: Project;
  topology: TopologySnapshot;
  templates: RuntimeTemplate[];
  runtimeInfo: RuntimeInfo | null;
  resources: ResourceMonitor | null;
  resourceError: string;
  refreshingResources: boolean;
  connection: "checking" | "connected" | "reconnecting" | "disconnected";
  batchLifecycleRunning: boolean;
  selection: Selection;
  selectedNode: Node | null | undefined;
  selectedLink: Link | null | undefined;
  connecting: boolean;
  linkEndpoints: LinkEndpoint[];
  onBack: () => void;
  onRefreshResources: () => void;
  onOpenNodeDialog: () => void;
  onOpenMarketplace: () => void;
  onOpenApplianceManager: () => void;
  onOpenPreferences: () => void;
  onAddTemplate: (templateId: string, x?: number, y?: number) => void;
  onNodeLifecycle: (id: string, action: LifecycleAction) => void;
  onStartAll: () => void;
  onStopAll: () => void;
  onOpenConsole: (node: Node, auxiliary?: boolean) => void;
  onToggleConnect: () => void;
  onCreateLink: () => void;
  onDeleteSelection: () => void;
  onNodeRename: (id: string, name: string) => void;
  onConfigureNode: (id: string, patch: Record<string, unknown>) => Promise<void>;
  onDuplicateNode: (node: Node) => void;
  onSaveTemplate: (id: string, patch: Record<string, unknown>) => Promise<void>;
  onSelectNode: (id: string) => void;
  onSelectLink: (id: string) => void;
  onSelectCanvas: () => void;
  onSelectPort: (endpoint: LinkEndpoint) => void;
  onNodeContext: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
};


export function SimulatorWorkspace(props: SimulatorWorkspaceProps) {
  const [view, setView] = useState({ zoom: 1, centerX: 0, centerY: 0 });
  const [topology, setTopology] = useState(props.topology);
  const [deviceQuery, setDeviceQuery] = useState("");
  const [summaryQuery, setSummaryQuery] = useState("");
  const [summaryStatus, setSummaryStatus] = useState("all");
  const [rightPanel, setRightPanel] = useState("devices");
  const [openTemplateGroups, setOpenTemplateGroups] = useState<Record<string, boolean>>({});
  const [configureNode, setConfigureNode] = useState<Node | null>(null);
  const [interfaceNode, setInterfaceNode] = useState<Node | null>(null);
  const [snapshotsOpen, setSnapshotsOpen] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const [screenshotOpen, setScreenshotOpen] = useState(false);
  const [linkPropertiesOpen, setLinkPropertiesOpen] = useState(false);
  const [symbolNode, setSymbolNode] = useState<Node | null>(null);
  const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>(props.selection?.kind === "node" ? [props.selection.id] : []);
  const [selectedDrawingId, setSelectedDrawingId] = useState<string | null>(null);
  const [drawingTool, setDrawingTool] = useState<DrawingTool | null>(null);
  const [pendingDrawing, setPendingDrawing] = useState<{ tool: DrawingTool; values: DrawingValues } | null>(null);
  const [drawingEditOpen, setDrawingEditOpen] = useState(false);
  const [drawingEditTool, setDrawingEditTool] = useState<DrawingTool | null>(null);
  const [drawingEditValues, setDrawingEditValues] = useState<DrawingValues | null>(null);
  const [gridVisible, setGridVisible] = useState(true);
  const [snapToGrid, setSnapToGrid] = useState(true);
  const [projectLocked, setProjectLocked] = useState(Boolean(props.project.locked));
  const [symbolDefaults, setSymbolDefaults] = useState<Record<string, string>>({});
  const [customSymbolIds, setCustomSymbolIds] = useState<string[]>([]);
  const [featureMessage, setFeatureMessage] = useState("");
  const selectedDrawing = topology.drawings.find((drawing) => drawing.drawing_id === selectedDrawingId) ?? null;
  const visibleSummaryNodes = useMemo(() => topology.nodes.filter((node) => {
    const matchesQuery = !summaryQuery.trim() || `${node.name} ${node.node_type ?? ""}`.toLowerCase().includes(summaryQuery.trim().toLowerCase());
    const matchesStatus = summaryStatus === "all" || (node.status ?? "stopped") === summaryStatus;
    return matchesQuery && matchesStatus;
  }), [summaryQuery, summaryStatus, topology.nodes]);
  const groupedTemplates = useMemo(() => {
    const query = deviceQuery.trim().toLowerCase();
    return ["dynamips", "qemu", "docker", "vpcs", "ethernet_switch", "ethernet_hub", "cloud", "nat"].map((kind) => ({
      kind,
      templates: props.templates.filter((template) => template.template_type === kind && (!query || `${template.name} ${template.template_type}`.toLowerCase().includes(query))),
    }));
  }, [deviceQuery, props.templates]);
  const canvasNodes = useMemo(() => spreadOverlappingNodes(topology.nodes), [topology.nodes]);
  const selectedNode = topology.nodes.find((node) => node.node_id === (props.selection?.kind === "node" ? props.selection.id : selectedNodeIds[0])) ?? props.selectedNode;
  const selectedLink = props.selection?.kind === "link" ? topology.links.find((link) => link.link_id === props.selection?.id) ?? props.selectedLink : props.selectedLink;
  const selectedLifecycleNodes = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && !node.locked);

  useEffect(() => { setTopology(props.topology); }, [props.topology]);
  useEffect(() => { setSelectedNodeIds(props.selection?.kind === "node" ? [props.selection.id] : []); }, [props.selection?.kind, props.selection?.id]);
  useEffect(() => { setProjectLocked(Boolean(props.project.locked)); }, [props.project.project_id, props.project.locked]);
  useEffect(() => {
    let active = true;
    void Promise.allSettled([api.listSymbols(), api.defaultSymbols()]).then(([customResult, defaultsResult]) => {
      if (!active) return;
      if (customResult.status === "fulfilled") {
        setCustomSymbolIds(customResult.value.flatMap((item) => {
          if (typeof item === "string") return [item];
          const id = item.symbol_id ?? item.filename ?? item.name;
          return typeof id === "string" ? [id] : [];
        }));
      }
      if (defaultsResult.status === "fulfilled") setSymbolDefaults(defaultsResult.value.symbols ?? {});
    });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!featureMessage) return;
    const timer = window.setTimeout(() => setFeatureMessage(""), 6000);
    return () => window.clearTimeout(timer);
  }, [featureMessage]);

  useEffect(() => {
    if (props.selection) setRightPanel("topology");
  }, [props.selection?.id, props.selection?.kind]);

  function fitTopology() {
    if (topology.nodes.length === 0) {
      setView({ zoom: 1, centerX: 0, centerY: 0 });
      return;
    }
    const xValues = canvasNodes.map((node) => Number(node.x ?? 0));
    const yValues = canvasNodes.map((node) => Number(node.y ?? 0));
    const minX = Math.min(...xValues);
    const maxX = Math.max(...xValues);
    const minY = Math.min(...yValues);
    const maxY = Math.max(...yValues);
    const zoom = Math.max(0.4, Math.min(maxZoom, Math.min(scene.width / (maxX - minX + 244), scene.height / (maxY - minY + 166))));
    setView({ zoom, centerX: (minX + maxX) / 2, centerY: (minY + maxY) / 2 });
  }

  function zoomToSelection() {
    const selectedIds = new Set(selectedNodeIds);
    if (props.selection?.kind === "node") selectedIds.add(props.selection.id);
    if (props.selection?.kind === "link") {
      const link = topology.links.find((item) => item.link_id === props.selection?.id);
      link?.nodes.forEach((endpoint) => selectedIds.add(endpoint.node_id));
    }
    const selectedNodes = canvasNodes.filter((node) => selectedIds.has(node.node_id));
    const boxes = selectedNodes.map((node) => ({ left: Number(node.x ?? 0) - 56, right: Number(node.x ?? 0) + 56, top: Number(node.y ?? 0) - 36, bottom: Number(node.y ?? 0) + 36 }));
    if (selectedDrawing) {
      const x = Number(selectedDrawing.x ?? 0);
      const y = Number(selectedDrawing.y ?? 0);
      boxes.push({ left: x, right: x + Number(selectedDrawing.width ?? 160), top: y, bottom: y + Number(selectedDrawing.height ?? 90) });
    }
    if (!boxes.length) { setFeatureMessage("Select a device, link, or drawing to zoom to it."); return; }
    const left = Math.min(...boxes.map((box) => box.left));
    const right = Math.max(...boxes.map((box) => box.right));
    const top = Math.min(...boxes.map((box) => box.top));
    const bottom = Math.max(...boxes.map((box) => box.bottom));
    const zoom = Math.max(0.4, Math.min(maxZoom, Math.min(scene.width / (right - left + 80), scene.height / (bottom - top + 80))));
    setView({ zoom, centerX: (left + right) / 2, centerY: (top + bottom) / 2 });
  }

  useEffect(() => {
    fitTopology();
  }, [props.project.project_id]);

  async function placeDrawing(point: { x: number; y: number }) {
    if (!pendingDrawing) { setSelectedDrawingId(null); setSelectedNodeIds([]); props.onSelectCanvas(); return; }
    try {
      const { tool, values } = pendingDrawing;
      const drawing = await api.createDrawing(props.project.project_id, {
        x: point.x, y: point.y, width: values.width, height: values.height, z_index: topology.drawings.length,
        drawing_kind: tool, svg: drawingSvg(tool, values),
      });
      setTopology((current) => ({ ...current, drawings: [...current.drawings, drawing] }));
      setSelectedDrawingId(drawing.drawing_id);
      setPendingDrawing(null);
      setFeatureMessage(`${tool[0]?.toUpperCase()}${tool.slice(1)} drawing added.`);
    } catch (reason) { setFeatureMessage(`Could not add drawing: ${messageFor(reason)}`); }
  }

  async function saveEditedDrawing(tool: DrawingTool, values: DrawingValues) {
    if (!selectedDrawing) return;
    try {
      const updated = await api.updateDrawing(props.project.project_id, selectedDrawing.drawing_id, {
        width: values.width, height: values.height, drawing_kind: tool, svg: drawingSvg(tool, values),
      });
      setTopology((current) => ({ ...current, drawings: current.drawings.map((drawing) => drawing.drawing_id === updated.drawing_id ? updated : drawing) }));
      setDrawingEditOpen(false);
    } catch (reason) { setFeatureMessage(`Could not edit drawing: ${messageFor(reason)}`); }
  }

  async function removeDrawing() {
    if (!selectedDrawing) return;
    try {
      await api.deleteDrawing(props.project.project_id, selectedDrawing.drawing_id);
      setTopology((current) => ({ ...current, drawings: current.drawings.filter((drawing) => drawing.drawing_id !== selectedDrawing.drawing_id) }));
      setSelectedDrawingId(null);
    } catch (reason) { setFeatureMessage(`Could not delete drawing: ${messageFor(reason)}`); }
  }

  async function moveDrawing(drawingId: string, x: number, y: number) {
    try {
      const drawing = await api.updateDrawing(props.project.project_id, drawingId, { x, y });
      setTopology((current) => ({ ...current, drawings: current.drawings.map((item) => item.drawing_id === drawingId ? drawing : item) }));
    } catch (reason) { setFeatureMessage(`Could not move drawing: ${messageFor(reason)}`); }
  }

  async function changeDrawingOrder(direction: "front" | "back") {
    if (!selectedDrawing) return;
    const reordered = reorderDrawings(topology.drawings, selectedDrawing.drawing_id, direction);
    try {
      const updated = await Promise.all(reordered.map((drawing, z_index) => api.updateDrawing(props.project.project_id, drawing.drawing_id, { z_index })));
      setTopology((current) => ({ ...current, drawings: current.drawings.map((drawing) => updated.find((item) => item.drawing_id === drawing.drawing_id) ?? drawing) }));
    } catch (reason) { setFeatureMessage(`Could not change drawing order: ${messageFor(reason)}`); }
  }

  async function toggleProjectLock() {
    const locked = !projectLocked;
    try {
      if (locked) await api.lockProject(props.project.project_id);
      else await api.unlockProject(props.project.project_id);
      setProjectLocked(locked); setFeatureMessage(locked ? "Project locked." : "Project unlocked.");
    }
    catch (reason) { setFeatureMessage(`Project lock is not available: ${messageFor(reason)}`); }
  }

  async function toggleNodeLock(node: Node) {
    const locked = !Boolean(node.locked);
    try {
      const updated = await api.updateNode(props.project.project_id, node.node_id, { locked });
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === node.node_id ? updated : item) }));
      setFeatureMessage(locked ? `${node.name} locked.` : `${node.name} unlocked.`);
    } catch (reason) { setFeatureMessage(`Node lock is not available: ${messageFor(reason)}`); }
  }

  async function toggleNodeIsolation(node: Node) {
    try {
      const updated = node.isolated ? await api.unisolateNode(props.project.project_id, node.node_id) : await api.isolateNode(props.project.project_id, node.node_id);
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === node.node_id ? updated : item) }));
      setFeatureMessage(updated.isolated ? `${node.name} isolated.` : `${node.name} unisolated.`);
    } catch (reason) { setFeatureMessage(`Node isolation is not available: ${messageFor(reason)}`); }
  }

  async function toggleLinkSuspend(link: Link) {
    try {
      const updated = await api.updateLink(props.project.project_id, link.link_id, { suspend: !link.suspend });
      setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === link.link_id ? updated : item) }));
      setFeatureMessage(updated.suspend ? "Link suspended." : "Link resumed.");
    } catch (reason) { setFeatureMessage(`Link suspend is not available: ${messageFor(reason)}`); }
  }

  async function saveNodeLabelPosition(nodeId: string, x: number, y: number) {
    const node = topology.nodes.find((item) => item.node_id === nodeId);
    if (!node) return;
    const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
    try {
      const updated = await api.updateNode(props.project.project_id, nodeId, { label: { ...label, x, y } });
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === nodeId ? updated : item) }));
    } catch (reason) { setFeatureMessage(`Could not save label position: ${messageFor(reason)}`); }
  }

  async function saveLinkEndpointLabelPosition(linkId: string, nodeId: string, x: number, y: number) {
    const link = topology.links.find((item) => item.link_id === linkId);
    const node = topology.nodes.find((item) => item.node_id === nodeId);
    if (!link || !node) return;
    try {
      const updated = await api.updateLink(props.project.project_id, linkId, {
        nodes: link.nodes.map((endpoint) => endpoint.node_id === nodeId
          ? { ...endpoint, label: { ...endpoint.label, text: endpoint.label?.text ?? portLabel(node, endpoint), x, y } }
          : endpoint),
      });
      setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === updated.link_id ? updated : item) }));
    } catch (reason) { setFeatureMessage(`Could not save endpoint label position: ${messageFor(reason)}`); }
  }

  async function exportScreenshot(request: ScreenshotRequest) {
    const svg = document.querySelector<SVGSVGElement>(".sim-canvas-surface svg");
    if (!svg) return;
    try {
      const svgText = serializeSvgScreenshot(svg);
      let blob: Blob;
      let extension: string;
      if (request.format === "png") {
        const rect = svg.getBoundingClientRect();
        const viewBox = svg.viewBox.baseVal;
        blob = await svgScreenshotPng(svgText, rect.width || viewBox.width, rect.height || viewBox.height);
        extension = "png";
      } else {
        blob = new Blob([svgText], { type: "image/svg+xml;charset=utf-8" });
        extension = "svg";
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url; link.download = `${safeFileName(request.name)}.${extension}`; link.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 0);
      setFeatureMessage(`Map screenshot exported as ${extension.toUpperCase()}.`);
    } catch (reason) { setFeatureMessage(`Could not export screenshot: ${messageFor(reason)}`); }
  }

  function runBulkLifecycle(action: LifecycleAction) {
    const targets = nodesForLifecycle(selectedLifecycleNodes, action);
    targets.forEach((node) => props.onNodeLifecycle(node.node_id, action));
    if (targets.length) setFeatureMessage(`${action[0]?.toUpperCase()}${action.slice(1)} requested for ${targets.length} selected device${targets.length === 1 ? "" : "s"}.`);
  }

  function deleteSelectedNodes() {
    const targets = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id));
    if (targets.length === 1 && props.selection?.kind === "node") { props.onDeleteSelection(); return; }
    void Promise.allSettled(targets.map((node) => api.deleteNode(props.project.project_id, node.node_id))).then((results) => {
      const deletedIds = new Set(results.flatMap((result, index) => result.status === "fulfilled" ? [targets[index]!.node_id] : []));
      const failed = results.length - deletedIds.size;
      const deleted = deletedIds.size;
      setTopology((current) => ({ ...current, nodes: current.nodes.filter((node) => !deletedIds.has(node.node_id)), links: current.links.filter((link) => !link.nodes.some((endpoint) => deletedIds.has(endpoint.node_id))) }));
      setSelectedNodeIds((current) => current.filter((id) => !deletedIds.has(id)));
      if (deletedIds.size === targets.length) props.onSelectCanvas();
      setFeatureMessage(failed ? `${deleted} devices deleted; ${failed} could not be deleted.` : `${deleted} devices deleted.`);
    });
  }

  function startDrawingTool(tool: DrawingTool) { setDrawingTool(tool); setPendingDrawing(null); }

  function onDrawingConfigured(tool: DrawingTool, values: DrawingValues) {
    if (drawingEditOpen) void saveEditedDrawing(tool, values);
    else { setPendingDrawing({ tool, values }); setFeatureMessage(`Click the map to place the ${tool} drawing.`); }
  }

  function selectLinkEndpoint(endpoint: LinkEndpoint) {
    const alreadySelected = props.linkEndpoints.some((item) => sameEndpoint(item, endpoint));
    const occupied = topology.links.some((link) => link.nodes.some((item) => sameEndpoint(item, endpoint)));
    if (occupied && !alreadySelected) {
      setFeatureMessage("That interface is already connected. Choose a free interface for this link.");
      return;
    }
    if (!alreadySelected && props.linkEndpoints.length >= 2) {
      setFeatureMessage("A GNS3 link connects exactly two interfaces. Create this link before selecting another pair.");
      return;
    }
    props.onSelectPort(endpoint);
  }

  return (
    <section className="sim-workspace" aria-label={`Project ${props.project.name}`}>
      <header className="sim-toolbar">
        <div className="sim-project-title">
          <Button variant="ghost" size="icon" onClick={props.onBack} aria-label="Back to projects"><ChevronDown className="rotate-90" /></Button>
          <div className="sim-project-mark"><Network size={17} /></div>
          <div className="min-w-0">
            <h1 title={props.project.name}>{props.project.name}</h1>
          </div>
          <Badge variant="outline" className="capitalize">{props.project.status ?? "opened"}</Badge>
        </div>
        <div className="sim-toolbar-actions">
          <Badge variant={props.connection === "connected" ? "secondary" : props.connection === "disconnected" ? "destructive" : "outline"} className="gap-1.5">
            <span className={`sim-connection-dot ${props.connection}`} />{props.connection === "connected" ? "Controller connected" : props.connection === "reconnecting" ? "Reconnecting" : props.connection === "checking" ? "Connecting" : "Controller unavailable"}
          </Badge>
          <Button variant="outline" onClick={props.onStartAll} disabled={props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "start").length} title="Start every stopped device"><Play />{props.batchLifecycleRunning ? "Working…" : "Start all"}</Button>
          <Button variant="outline" onClick={props.onStopAll} disabled={props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "stop").length} title="Stop every running device"><Square />Stop all</Button>
          <Separator orientation="vertical" className="h-7" />
          <Button variant={props.connecting ? "default" : "outline"} aria-label={props.connecting ? "Cancel link" : "Connect ports"} aria-pressed={props.connecting} onClick={props.onToggleConnect}><Cable />{props.connecting ? "Cancel link" : "Draw link"}</Button>
          {props.connecting && <Button onClick={props.onCreateLink} disabled={props.linkEndpoints.length < 2}><Link2 />Create link{props.linkEndpoints.length > 0 ? ` · ${props.linkEndpoints.length}` : ""}</Button>}
          {selectedNodeIds.length > 1 && <>
            <Badge variant="secondary">{selectedNodeIds.length} selected</Badge>
            {nodesForLifecycle(selectedLifecycleNodes, "start").length > 0 && <Button size="sm" variant="outline" onClick={() => runBulkLifecycle("start")}><Play />Start</Button>}
            {nodesForLifecycle(selectedLifecycleNodes, "stop").length > 0 && <Button size="sm" variant="outline" onClick={() => runBulkLifecycle("stop")}><Square />Stop</Button>}
            {nodesForLifecycle(selectedLifecycleNodes, "suspend").length > 0 && <Button size="sm" variant="outline" onClick={() => runBulkLifecycle("suspend")}><Pause />Suspend</Button>}
            {nodesForLifecycle(selectedLifecycleNodes, "reload").length > 0 && <Button size="sm" variant="outline" onClick={() => runBulkLifecycle("reload")}><Activity />Reload</Button>}
              <Button size="sm" variant="outline" onClick={() => topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && node.status === "started").forEach((node) => props.onOpenConsole(node))}><Terminal />Consoles</Button>
            <Button size="sm" variant="destructive" onClick={deleteSelectedNodes}><Trash2 />Delete</Button>
          </>}
          <DropdownMenu>
            <DropdownMenuTrigger asChild><Button variant="outline" size="icon" aria-label="Project map menu" title="Project map menu"><MoreHorizontal /></Button></DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-60">
              <DropdownMenuLabel>Project map</DropdownMenuLabel>
              <DropdownMenuItem onSelect={props.onOpenPreferences}><Settings2 />Preferences</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void toggleProjectLock()}>{projectLocked ? <Unlock /> : <Lock />}{projectLocked ? "Unlock project" : "Lock project"}</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setGridVisible((value) => !value)}>{gridVisible ? "Hide grid" : "Show grid"}</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setSnapToGrid((value) => !value)}>{snapToGrid ? "Disable snap to grid" : "Snap to grid"}</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Add drawing</DropdownMenuLabel>
              <DropdownMenuItem onSelect={() => startDrawingTool("rectangle")}><RectangleHorizontal />Rectangle</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => startDrawingTool("ellipse")}><Circle />Ellipse</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => startDrawingTool("line")}><Minus />Line</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => startDrawingTool("text")}><Type />Text</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => startDrawingTool("image")}><Image />Image</DropdownMenuItem>
              {selectedDrawing && <><DropdownMenuSeparator /><DropdownMenuItem onSelect={() => { setDrawingEditTool((selectedDrawing.drawing_kind as DrawingTool) ?? "rectangle"); setDrawingEditValues(valuesFromDrawing(selectedDrawing)); setDrawingEditOpen(true); }}>Edit selected drawing</DropdownMenuItem><DropdownMenuItem onSelect={() => void changeDrawingOrder("front")}><Layers />Bring drawing forward</DropdownMenuItem><DropdownMenuItem onSelect={() => void changeDrawingOrder("back")}><Layers />Send drawing backward</DropdownMenuItem><DropdownMenuItem onSelect={() => void removeDrawing()}><Trash2 />Delete drawing</DropdownMenuItem></>}
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => setSnapshotsOpen(true)}>Project snapshots…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setLogOpen(true)}>Log console…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setScreenshotOpen(true)}><Camera />Take a screenshot…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setHelpOpen(true)}><HelpCircle />Map shortcuts</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => setInfoOpen(true)}>Project information…</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button variant="outline" onClick={props.onOpenMarketplace}><Search />Image catalog</Button>
          <Button variant="outline" onClick={props.onOpenApplianceManager}><Settings2 />Appliances</Button>
        </div>
      </header>

      <div className="sim-workspace-grid">
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
                  {groupedTemplates.filter(({ templates }) => !deviceQuery.trim() || templates.length > 0).map(({ kind, templates }) => <Collapsible key={kind} open={openTemplateGroups[kind] ?? templates.length > 0} onOpenChange={(open) => setOpenTemplateGroups((current) => ({ ...current, [kind]: open }))}>
                    <CollapsibleTrigger className="sim-group-trigger"><span>{deviceGroupLabel(kind)}</span><Badge variant="secondary">{templates.length}</Badge><ChevronDown className="ml-auto size-4 text-muted-foreground" /></CollapsibleTrigger>
                    <CollapsibleContent className="space-y-1 pt-1">
                      {templates.map((template) => <DeviceTemplateCard key={template.template_id} template={template} onAdd={() => props.onAddTemplate(template.template_id)} onSave={props.onSaveTemplate} />)}
                    </CollapsibleContent>
                  </Collapsible>)}
                  {deviceQuery.trim() && props.templates.length > 0 && groupedTemplates.every(({ templates }) => templates.length === 0) && <p className="px-2 py-2 text-xs text-muted-foreground">No device templates match “{deviceQuery.trim()}”.</p>}
                  {props.templates.length === 0 && <Card className="p-3 text-xs leading-relaxed text-muted-foreground">No device templates are available yet. Import a Cisco IOS or QEMU image, pull a Docker image, or install VPCS. Images are stored in the global library.</Card>}
                </div>
              </ScrollArea>
              <div className="sim-sidebar-bottom-actions"><Button variant="outline" className="w-full" onClick={props.onOpenMarketplace}><Search />Find devices and images</Button></div>
            </TabsContent>

            <TabsContent value="topology" className="sim-sidebar-content">
              <div className="sim-sidebar-section-title"><div><h2>Topology summary</h2><p>Device state and interfaces.</p></div><Badge variant="outline">{visibleSummaryNodes.length}/{topology.nodes.length}</Badge></div>
              <div className="grid grid-cols-[1fr_112px] gap-2 px-3 pb-3"><label className="relative"><Filter className="pointer-events-none absolute left-2.5 top-2.5 size-3.5 text-muted-foreground" /><Input aria-label="Filter topology devices" placeholder="Filter devices…" value={summaryQuery} onChange={(event) => setSummaryQuery(event.target.value)} className="h-8 pl-8 text-xs" /></label><Select value={summaryStatus} onValueChange={setSummaryStatus}><SelectTrigger aria-label="Filter devices by state" className="h-8 text-xs"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All states</SelectItem><SelectItem value="started">Running</SelectItem><SelectItem value="stopped">Stopped</SelectItem><SelectItem value="suspended">Suspended</SelectItem></SelectContent></Select></div>
              <ScrollArea className="sim-topology-list-scroll">
                {topology.nodes.length ? <div className="px-3 pb-3">
                  <div className="overflow-hidden rounded-md border">
                    <Table className="sim-node-table">
                      <TableHeader><TableRow><TableHead>Device</TableHead><TableHead className="w-24">State</TableHead></TableRow></TableHeader>
                      <TableBody>{visibleSummaryNodes.map((node) => <TableRow key={node.node_id} data-state={props.selection?.kind === "node" && props.selection.id === node.node_id ? "selected" : undefined} className="cursor-pointer" onClick={() => { setSelectedNodeIds([node.node_id]); props.onSelectNode(node.node_id); }}>
                        <TableCell><div className="flex min-w-0 items-center gap-2"><span className={`sim-state-dot ${node.status === "started" ? "running" : ""}`} /><span className="truncate">{node.name}</span></div></TableCell>
                        <TableCell><Badge variant={node.status === "started" ? "default" : "secondary"} className="capitalize">{node.status ?? "stopped"}</Badge></TableCell>
                      </TableRow>)}</TableBody>
                    </Table>
                  </div>
                  {!visibleSummaryNodes.length && <p className="px-3 py-4 text-center text-xs text-muted-foreground">No devices match these filters.</p>}
                </div> : <p className="px-5 py-8 text-center text-sm text-muted-foreground">No devices on this topology.</p>}
                {selectedNode && <NodeInspector key={selectedNode.node_id}
                  node={selectedNode}
                  links={topology.links}
                  nodes={topology.nodes}
                  onLifecycle={props.onNodeLifecycle}
                  onOpenConsole={props.onOpenConsole}
                  onRename={props.onNodeRename}
                  onConfigure={() => setConfigureNode(selectedNode)}
                  onDelete={props.onDeleteSelection}
                />}
                {selectedLink && <Card className="mx-3 mb-3 p-3">
                  <div className="flex items-center justify-between"><div><p className="text-sm font-medium">Ethernet segment</p><p className="mt-1 text-xs text-muted-foreground">{selectedLink.nodes.length} connected interfaces</p></div><Button variant="ghost" size="icon-sm" onClick={props.onDeleteSelection} aria-label="Delete selected link"><Trash2 /></Button></div>
                  <div className="mt-2 flex flex-wrap gap-2"><Button size="sm" variant="outline" onClick={() => setLinkPropertiesOpen(true)}><Settings2 />Link properties</Button><Button size="sm" variant="outline" onClick={() => void toggleLinkSuspend(selectedLink)}>{selectedLink.suspend ? "Resume link" : "Suspend link"}</Button></div>
                  <Separator className="my-3" />
                  <div className="space-y-2">{selectedLink.nodes.map((endpoint) => { const node = topology.nodes.find((item) => item.node_id === endpoint.node_id); return <div key={`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`} className="flex justify-between gap-3 text-xs"><span className="truncate">{node?.name ?? endpoint.node_id}</span><span className="font-mono text-muted-foreground">{portLabel(node, endpoint)}</span></div>; })}</div>
                </Card>}
              </ScrollArea>
              <div className="sim-topology-summary"><div><span>Links</span><b>{topology.links.length}</b></div><div><span>Drawing objects</span><b>{topology.drawings.length}</b></div><div><span>Controller events</span><b>#{topology.event_cursor}</b></div></div>
            </TabsContent>
          </Tabs>
        </aside>
        <Card className="sim-canvas-card">
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div className={`sim-canvas ${props.connecting ? "is-connecting" : ""} ${pendingDrawing ? "is-drawing" : ""}`}>
                {featureMessage && <div className="sim-map-inline-notice" role="status">{featureMessage}</div>}
                <div className="sim-canvas-controls">
                  <div className="sim-map-status">
                    <div className="sim-map-count"><span className="sim-live-dot" />{topology.nodes.length} devices <span aria-hidden="true">·</span> {topology.links.length} links{projectLocked ? <><span aria-hidden="true">·</span><Lock size={12} />Locked</> : null}</div>
                    {props.connecting && <div className="sim-link-hint"><Cable size={14} /><span>Select ports to build a link.</span><Badge variant="secondary">{props.linkEndpoints.length} selected</Badge></div>}
                    {pendingDrawing && <div className="sim-link-hint"><Pencil size={14} /><span>Click to place {pendingDrawing.tool}.</span><Button size="xs" variant="ghost" onClick={() => setPendingDrawing(null)}>Cancel</Button></div>}
                  </div>
                  <div className="sim-map-actions">
                    <ResourceMonitorPopover compact resources={props.resources} runtimeInfo={props.runtimeInfo} error={props.resourceError} refreshing={props.refreshingResources} onRefresh={props.onRefreshResources} />
                    <Button variant="ghost" size="icon-sm" aria-label="Zoom out" onClick={() => setView((current) => ({ ...current, zoom: Math.max(0.4, Number((current.zoom - 0.1).toFixed(1))) }))}><ZoomOut /></Button>
                    <span className="w-12 text-center font-mono text-xs text-muted-foreground">{Math.round(view.zoom * 100)}%</span>
                    <Button variant="ghost" size="icon-sm" aria-label="Zoom in" onClick={() => setView((current) => ({ ...current, zoom: Math.min(maxZoom, Number((current.zoom + 0.1).toFixed(1))) }))}><ZoomIn /></Button>
                    <Button variant="ghost" size="icon-sm" aria-label="Fit topology" title="Fit all devices" onClick={fitTopology}><Scan /></Button>
                    <Button variant="ghost" size="icon-sm" aria-label="Zoom to selection" title="Zoom to selected devices, link, or drawing" onClick={zoomToSelection}><Scan /></Button>
                    <Button variant="ghost" size="sm" aria-label="Reset zoom" title="Reset zoom to 100%" onClick={() => setView({ zoom: 1, centerX: 0, centerY: 0 })}>100%</Button>
                  </div>
                </div>
                <TopologyCanvas
                  nodes={canvasNodes}
                  links={topology.links}
                  drawings={topology.drawings}
                  symbolDefaults={symbolDefaults}
                  customSymbolIds={customSymbolIds}
                  zoom={view.zoom}
                  centerX={view.centerX}
                  centerY={view.centerY}
                  gridVisible={gridVisible}
                  snapToGrid={snapToGrid}
                  projectLocked={projectLocked}
                  selectedNodeIds={selectedNodeIds}
                  selectedDrawingId={selectedDrawingId}
                  onCenterChange={(centerX, centerY) => setView((current) => ({ ...current, centerX, centerY }))}
                  selection={props.selection}
                  connecting={props.connecting}
                  linkEndpoints={props.linkEndpoints}
                  onNodeClick={(id) => { if (props.connecting) setInterfaceNode(topology.nodes.find((node) => node.node_id === id) ?? null); else { setSelectedDrawingId(null); setSelectedNodeIds([id]); props.onSelectNode(id); } }}
                  onMultiSelectNode={(id) => setSelectedNodeIds((current) => current.includes(id) ? current.filter((item) => item !== id) : [...current, id])}
                  onMultiSelect={(ids) => { setSelectedDrawingId(null); setSelectedNodeIds(ids); }}
                  onLinkClick={(id) => { setSelectedDrawingId(null); setSelectedNodeIds([]); props.onSelectLink(id); }}
                  onCanvasClick={(point) => { void placeDrawing(point); }}
                  onPortClick={selectLinkEndpoint}
                  onNodeContext={(id) => { setSelectedNodeIds([id]); setSelectedDrawingId(null); props.onNodeContext(id); }}
                  onNodeMove={props.onNodeMove}
                  onNodeMoveEnd={props.onNodeMoveEnd}
                  onDrawingClick={(id) => { setSelectedDrawingId(id); setSelectedNodeIds([]); props.onSelectCanvas(); }}
                  onDrawingMove={(id, x, y) => void moveDrawing(id, x, y)}
                  onDeleteSelected={() => { if (selectedDrawing) void removeDrawing(); else if (selectedLink) props.onDeleteSelection(); else deleteSelectedNodes(); }}
                  onClearSelection={() => { setSelectedNodeIds([]); setSelectedDrawingId(null); props.onSelectCanvas(); }}
                  onZoom={(delta) => setView((current) => ({ ...current, zoom: Math.max(0.4, Math.min(maxZoom, Number((current.zoom + delta).toFixed(1)))) }))}
                  onNodeLabelMove={(id, x, y) => void saveNodeLabelPosition(id, x, y)}
                  onLinkEndpointLabelMove={(linkId, nodeId, x, y) => void saveLinkEndpointLabelPosition(linkId, nodeId, x, y)}
                  onDropTemplate={(templateId, x, y) => props.onAddTemplate(templateId, x, y)}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent className="w-56">
              <ContextMenuLabel>{selectedNode?.name ?? (selectedLink ? "Ethernet link" : selectedDrawing ? "Drawing" : "Topology")}</ContextMenuLabel>
              <ContextMenuSeparator />
              {selectedNode ? <>
                {canApplyLifecycle(selectedNode, "start") && <ContextMenuItem disabled={Boolean(selectedNode.runtime_missing)} onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "start")}><Play />{selectedNode.status === "suspended" ? "Resume device" : "Start device"}</ContextMenuItem>}
                {canApplyLifecycle(selectedNode, "stop") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "stop")}><Power />Stop device</ContextMenuItem>}
                {canApplyLifecycle(selectedNode, "suspend") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "suspend")}><Pause />Suspend device</ContextMenuItem>}
                {canApplyLifecycle(selectedNode, "reload") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "reload")}><Activity />Reload device</ContextMenuItem>}
                {selectedNode.status === "started" && <ContextMenuItem onSelect={() => props.onOpenConsole(selectedNode)}><Terminal />Open console</ContextMenuItem>}
                {selectedNode.status === "started" && nodeHasAuxConsole(selectedNode) && <ContextMenuItem onSelect={() => props.onOpenConsole(selectedNode, true)}><Terminal />Open auxiliary console</ContextMenuItem>}
                <ContextMenuItem onSelect={() => { document.querySelector<HTMLButtonElement>("[data-tab=topology]")?.click(); window.setTimeout(() => document.querySelector<HTMLInputElement>("[data-rename-node]")?.focus(), 0); }}><Pencil />Edit device name</ContextMenuItem>
                {["qemu", "docker", "vpcs", "dynamips"].includes(selectedNode.node_type ?? "") && <ContextMenuItem onSelect={() => setConfigureNode(selectedNode)}><Settings2 />Configure device</ContextMenuItem>}
                <ContextMenuItem onSelect={() => setSymbolNode(selectedNode)}><Image />Change symbol</ContextMenuItem>
                <ContextMenuItem onSelect={() => void toggleNodeIsolation(selectedNode)}>{selectedNode.isolated ? <Unlock /> : <Lock />}{selectedNode.isolated ? "Unisolate device" : "Isolate device"}</ContextMenuItem>
                <ContextMenuItem onSelect={() => void toggleNodeLock(selectedNode)}>{selectedNode.locked ? <Unlock /> : <Lock />}{selectedNode.locked ? "Unlock device" : "Lock device"}</ContextMenuItem>
                <ContextMenuItem onSelect={() => props.onDuplicateNode(selectedNode)}><Plus />Duplicate device</ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem variant="destructive" onSelect={props.onDeleteSelection}><Trash2 />Delete device</ContextMenuItem>
              </> : selectedLink ? <>
                <ContextMenuItem onSelect={() => setLinkPropertiesOpen(true)}><Settings2 />Link properties and filters</ContextMenuItem>
                <ContextMenuItem onSelect={() => void toggleLinkSuspend(selectedLink)}>{selectedLink.suspend ? <Play /> : <Pause />}{selectedLink.suspend ? "Resume link" : "Suspend link"}</ContextMenuItem>
                <ContextMenuItem onSelect={() => setLinkPropertiesOpen(true)}><Activity />Packet capture…</ContextMenuItem>
                <ContextMenuSeparator /><ContextMenuItem variant="destructive" onSelect={props.onDeleteSelection}><Trash2 />Delete link</ContextMenuItem>
              </> : selectedDrawing ? <>
                <ContextMenuItem onSelect={() => { setDrawingEditTool((selectedDrawing.drawing_kind as DrawingTool) ?? "rectangle"); setDrawingEditValues(valuesFromDrawing(selectedDrawing)); setDrawingEditOpen(true); }}><Pencil />Edit drawing</ContextMenuItem>
                <ContextMenuItem onSelect={() => void changeDrawingOrder("front")}><Layers />Bring forward</ContextMenuItem>
                <ContextMenuItem onSelect={() => void changeDrawingOrder("back")}><Layers />Send backward</ContextMenuItem>
                <ContextMenuItem variant="destructive" onSelect={() => void removeDrawing()}><Trash2 />Delete drawing</ContextMenuItem>
              </> : <ContextMenuItem onSelect={props.onOpenNodeDialog}><Plus />Import or configure device</ContextMenuItem>}
            </ContextMenuContent>
          </ContextMenu>
        </Card>

      </div>
      <NodeSettingsDialog node={configureNode} open={configureNode !== null} onOpenChange={(open) => { if (!open) setConfigureNode(null); }} onSave={props.onConfigureNode} />
      <InterfacePickerDialog node={interfaceNode} links={topology.links} open={interfaceNode !== null} onOpenChange={(open) => { if (!open) setInterfaceNode(null); }} onPick={selectLinkEndpoint} />
      <DrawingEditorDialog tool={drawingEditOpen ? drawingEditTool : drawingTool} open={drawingEditOpen || drawingTool !== null} mode={drawingEditOpen ? "edit" : "create"} initial={drawingEditValues} onOpenChange={(open) => { if (!open) { setDrawingTool(null); setDrawingEditOpen(false); } }} onSave={onDrawingConfigured} />
      <SnapshotsDialog projectId={props.project.project_id} open={snapshotsOpen} onOpenChange={setSnapshotsOpen} onRestored={setTopology} />
      <LinkPropertiesDialog projectId={props.project.project_id} link={selectedLink ?? null} nodes={topology.nodes} open={linkPropertiesOpen} onOpenChange={setLinkPropertiesOpen} onUpdated={(updated) => setTopology((current) => ({ ...current, links: current.links.map((link) => link.link_id === updated.link_id ? updated : link) }))} />
      <SymbolPickerDialog projectId={props.project.project_id} node={symbolNode} open={symbolNode !== null} onOpenChange={(open) => { if (!open) setSymbolNode(null); }} onSaved={(updated) => setTopology((current) => ({ ...current, nodes: current.nodes.map((node) => node.node_id === updated.node_id ? updated : node) }))} />
      <LogConsoleDialog projectId={props.project.project_id} cursor={topology.event_cursor} open={logOpen} onOpenChange={setLogOpen} />
      <HelpDialog open={helpOpen} onOpenChange={setHelpOpen} />
      <InfoDialog project={props.project} nodeCount={topology.nodes.length} linkCount={topology.links.length} drawingCount={topology.drawings.length} eventCursor={topology.event_cursor} open={infoOpen} onOpenChange={setInfoOpen} />
      <ScreenshotDialog open={screenshotOpen} onOpenChange={setScreenshotOpen} onExport={(request) => void exportScreenshot(request)} pngAvailable={pngScreenshotAvailable()} />
    </section>
  );
}

