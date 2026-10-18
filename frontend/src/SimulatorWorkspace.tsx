import { useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent, FormEvent, PointerEvent } from "react";
import {
  Activity, Cable, Check, ChevronDown, Cloud, Cpu, ExternalLink, HardDrive, HelpCircle, Image, Link2,
  ListTree, Monitor, Network, Pause, Pencil, Play, Plus, Power, Router, Search,
  Scan, Server, Settings2, Square, Terminal, Trash2, ZoomIn, ZoomOut, Lock, Unlock,
  MoreHorizontal, Camera, RectangleHorizontal, Circle, Minus, Type, Filter, Layers,
} from "lucide-react";
import type { ComponentType } from "react";
import type { Drawing, Link, LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { api, dockerHttpConsoleUrl } from "./api";
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
import { sanitizeDrawingSvg } from "./topology";
import { ResourceMonitorPopover } from "./ResourceMonitorPopover";
import { TemplateSettingsDialog } from "./TemplateSettingsDialog";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { DrawingEditorDialog, HelpDialog, InfoDialog, InterfacePickerDialog, LinkPropertiesDialog, LogConsoleDialog, ScreenshotDialog, SnapshotsDialog, SymbolPickerDialog } from "./map/MapDialogs";
import type { ScreenshotRequest } from "./map/MapDialogs";
import { drawingSvg, reorderDrawings, selectionBoxIds, serializeSvgScreenshot, svgScreenshotPng } from "./map/helpers";
import type { DrawingTool } from "./map/helpers";
import { resolveCanvasSymbol } from "./map/symbols";

type Selection = { kind: "node" | "link"; id: string } | null;
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

const scene = { width: 1440, height: 920 };
const maxZoom = 3;

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
  const selectedNode = props.selection?.kind === "node" ? topology.nodes.find((node) => node.node_id === props.selection?.id) ?? props.selectedNode : props.selectedNode;
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

function pngScreenshotAvailable() {
  const canvas = document.createElement("canvas");
  return typeof canvas.toBlob === "function" && typeof canvas.toDataURL === "function";
}

function spreadOverlappingNodes(nodes: Node[]) {
  const positions = new Map<string, number>();
  for (const node of nodes) {
    const key = `${Number(node.x ?? 0)}:${Number(node.y ?? 0)}`;
    positions.set(key, (positions.get(key) ?? 0) + 1);
  }
  const columns = Math.min(3, Math.max(1, Math.ceil(Math.sqrt(nodes.length))));
  return nodes.map((node, index) => {
    const x = Number(node.x ?? 0);
    const y = Number(node.y ?? 0);
    if ((positions.get(`${x}:${y}`) ?? 0) < 2) return node;
    const column = index % columns;
    const row = Math.floor(index / columns);
    return { ...node, x: x + (column - (columns - 1) / 2) * 150, y: y + row * 120 };
  });
}

function DeviceTemplateCard({ template, onAdd, onSave }: { template: RuntimeTemplate; onAdd: () => void; onSave: (id: string, patch: Record<string, unknown>) => Promise<void> }) {
  const Icon = iconForType(template.template_type);
  const [settingsOpen, setSettingsOpen] = useState(false);
  return <>
  <div className="sim-device-template" draggable onDragStart={(event) => {
    event.dataTransfer.effectAllowed = "copy";
    event.dataTransfer.setData("application/x-network-simulator-template", template.template_id);
    event.dataTransfer.setData("text/plain", template.name);
  }}>
    <span className="sim-template-icon"><Icon size={17} /></span>
    <span className="sim-template-copy"><b title={template.name}>{template.name}</b><small>{templateDescription(template)}</small></span>
    <Button variant="ghost" size="icon-xs" onClick={() => setSettingsOpen(true)} aria-label={`Configure ${template.name} template`} title="Configure template"><Settings2 /></Button>
    <Button variant="ghost" size="icon-xs" onClick={onAdd} aria-label={`Add ${template.name}`} title="Add to topology"><Plus /></Button>
  </div>
    <TemplateSettingsDialog template={template} open={settingsOpen} onOpenChange={setSettingsOpen} onSave={onSave} />
  </>;
}

function NodeSettingsDialog({ node, open, onOpenChange, onSave }: {
  node: Node | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (id: string, patch: Record<string, unknown>) => Promise<void>;
}) {
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const kind = node?.node_type;
  const supported = ["qemu", "docker", "vpcs", "dynamips"].includes(kind ?? "");

  useEffect(() => {
    if (!node) return;
    setDraft({
      name: node.name,
      memory: String(node.memory ?? (kind === "qemu" ? 512 : 0)),
      cpus: String(node.cpus ?? (kind === "qemu" ? 1 : 0)),
      adapters: String(node.adapters ?? 1),
      ...(kind === "qemu" ? {
        adapter_type: String(node.adapter_type ?? "e1000"),
        hda_disk_interface: String(node.hda_disk_interface ?? "ide"),
        boot_priority: String(node.boot_priority ?? "c"),
      } : {}),
      ...(kind === "docker" ? {
        start_command: String(node.start_command ?? ""),
        environment: String(node.environment ?? ""),
        extra_hosts: String(node.extra_hosts ?? ""),
        console_type: String(node.console_type ?? "telnet"),
        console_http_port: String(node.console_http_port ?? 80),
        console_http_path: String(node.console_http_path ?? "/"),
        aux_type: String(node.aux_type ?? "none"),
        extra_volumes: Array.isArray(node.extra_volumes)
          ? node.extra_volumes.filter((path): path is string => typeof path === "string").join("\n")
          : "",
      } : {}),
      ...(kind === "vpcs" ? { startup_script: String(node.startup_script ?? "") } : {}),
      ...(kind === "dynamips" ? { startup_config: String(node.startup_config ?? "") } : {}),
    });
    setError("");
  }, [kind, node, open]);

  const field = (name: string, label: string, type = "text", helper?: string, step?: string) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Input id={`node-${node?.node_id}-${name}`} type={type} step={step} min={type === "number" ? "0" : undefined} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} />
    {helper && <span className="text-xs text-muted-foreground">{helper}</span>}
  </label>;
  const selectField = (name: string, label: string, values: string[]) => <label className="grid gap-1.5" key={name}>
    <Label>{label}</Label>
    <Select value={draft[name] ?? values[0]} onValueChange={(value) => setDraft((current) => ({ ...current, [name]: value }))}>
      <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent>{values.map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent>
    </Select>
  </label>;
  const textArea = (name: string, label: string, helper: string, rows = 4) => <label className="grid gap-1.5" key={name}>
    <Label htmlFor={`node-${node?.node_id}-${name}`}>{label}</Label>
    <Textarea id={`node-${node?.node_id}-${name}`} value={draft[name] ?? ""} onChange={(event) => setDraft((current) => ({ ...current, [name]: event.target.value }))} rows={rows} />
    <span className="text-xs text-muted-foreground">{helper}</span>
  </label>;

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!node || !supported) return;
    setSaving(true);
    setError("");
    const patch: Record<string, unknown> = { name: draft.name?.trim() };
    if (kind === "qemu") {
      patch.memory = Number(draft.memory);
      patch.cpus = Number(draft.cpus);
      patch.adapters = Number(draft.adapters);
      patch.adapter_type = draft.adapter_type;
      patch.hda_disk_interface = draft.hda_disk_interface;
      patch.boot_priority = draft.boot_priority;
    } else if (kind === "docker") {
      patch.memory = Number(draft.memory);
      patch.cpus = Number(draft.cpus);
      patch.adapters = Number(draft.adapters);
      patch.start_command = draft.start_command ?? "";
      patch.environment = draft.environment ?? "";
      patch.extra_hosts = draft.extra_hosts ?? "";
      patch.extra_volumes = (draft.extra_volumes ?? "").split("\n").map((path) => path.trim()).filter(Boolean);
      patch.console_type = draft.console_type ?? "telnet";
      if (patch.console_type !== "none") {
        patch.console_http_port = Number(draft.console_http_port);
        patch.console_http_path = draft.console_http_path || "/";
      }
      patch.aux_type = draft.aux_type ?? "none";
    } else if (kind === "vpcs") {
      patch.startup_script = draft.startup_script ?? "";
    } else if (kind === "dynamips") {
      patch.startup_config = draft.startup_config ?? "";
    }
    try {
      await onSave(node.node_id, patch);
      onOpenChange(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not save device settings");
    } finally {
      setSaving(false);
    }
  }

  function openHttpConsole() {
    if (!node || (draft.console_type !== "http" && draft.console_type !== "https")) return;
    const host = typeof node.console_host === "string" && node.console_host ? node.console_host : window.location.hostname;
    const url = dockerHttpConsoleUrl(draft.console_type, host, Number(draft.console_http_port), draft.console_http_path || "/");
    if (!url) {
      setError("Enter a valid HTTP console port and path before opening the console.");
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  }

  if (!node || !supported) return null;
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="template-settings-dialog">
    <DialogHeader><DialogTitle>Configure {node.name}</DialogTitle><DialogDescription>
      Settings apply to this device only. Stop it before saving.{kind === "docker" ? " Docker settings replace its stopped container; configured persistent paths keep their data." : ""}
    </DialogDescription></DialogHeader>
    <form className="template-settings-form" onSubmit={(event) => void save(event)}>
      <div className="template-settings-grid">
        {field("name", "Device name")}
        {(kind === "qemu" || kind === "docker") && field("adapters", "Network adapters", "number", "The count cannot be reduced below ports already used by links.")}
        {(kind === "qemu" || kind === "docker") && field("memory", kind === "qemu" ? "RAM (MB)" : "Memory limit (MB)", "number", kind === "docker" ? "0 means no Docker memory limit." : undefined)}
        {(kind === "qemu" || kind === "docker") && field("cpus", kind === "qemu" ? "vCPUs" : "CPU limit", "number", kind === "docker" ? "0 means no Docker CPU limit." : undefined, kind === "docker" ? "0.25" : "1")}
        {kind === "qemu" && <>
          {selectField("adapter_type", "Network adapter model", ["e1000", "virtio-net-pci", "rtl8139", "vmxnet3"])}
          {selectField("hda_disk_interface", "Primary disk interface", ["ide", "scsi", "virtio", "sd"])}
          {selectField("boot_priority", "Boot priority", ["c", "d", "n", "dc", "cd"])}
        </>}
        {kind === "docker" && <div className="template-settings-wide">
          {selectField("console_type", "Console type", ["telnet", "vnc", "http", "https", "none"])}
          {draft.console_type === "http" || draft.console_type === "https" ? <div className="grid gap-2 rounded-md border p-3 sm:col-span-2">
            <p className="text-sm">The controller stores this console port and path but does not publish or proxy Docker ports. Open in a new tab assumes the service is reachable from your browser.</p>
            <div className="grid gap-3 sm:grid-cols-2">{field("console_http_port", "HTTP port in the container", "number", undefined, "1")}{field("console_http_path", "HTTP path", "text", "Must begin with a slash.")}</div>
            <Button type="button" variant="outline" className="w-fit" onClick={openHttpConsole}><ExternalLink />Open in new tab</Button>
          </div> : null}
          {draft.console_type === "vnc" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">The controller can start VNC on its TCP console port, but this UI has no browser VNC route.</p>}
          {draft.console_type === "none" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">The controller does not accept Docker console type “none” yet. Select another type to save.</p>}
          {selectField("aux_type", "Auxiliary console type", ["none", "telnet", "ssh"])}
          {draft.aux_type !== "none" && <p role="status" className="text-sm text-muted-foreground sm:col-span-2">When the device is running, open its context menu and choose “Open auxiliary console”.</p>}
          {textArea("start_command", "Start command", "Arguments passed to the image. Leave blank to use its default command.", 2)}
          {textArea("environment", "Environment variables", "One KEY=VALUE entry per line.")}
          {textArea("extra_hosts", "Extra hosts", "One host:address entry per line.")}
          {textArea("extra_volumes", "Persistent container paths", "One absolute path inside the container per line. Data lives in the project runtime directory.")}
        </div>}
        {kind === "vpcs" && <div className="template-settings-wide">{textArea("startup_script", "Startup script", "VPCS commands run when this device starts.")}</div>}
        {kind === "dynamips" && <div className="template-settings-wide">{textArea("startup_config", "Startup configuration", "IOS configuration written before the router starts.", 8)}</div>}
      </div>
      {node.status !== "stopped" && <p role="status" className="text-sm text-muted-foreground">Stop this device to edit its runtime settings.</p>}
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <DialogFooter><Button type="button" variant="ghost" disabled={saving} onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={saving || node.status !== "stopped" || (kind === "docker" && draft.console_type === "none")}>{saving ? "Saving…" : "Save device settings"}</Button></DialogFooter>
    </form>
  </DialogContent></Dialog>;
}

function NodeInspector({ node, links, nodes, onLifecycle, onOpenConsole, onRename, onConfigure, onDelete }: {
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

type CanvasProps = {
  nodes: Node[];
  links: Link[];
  drawings: Drawing[];
  symbolDefaults: Record<string, string>;
  customSymbolIds: string[];
  zoom: number;
  centerX: number;
  centerY: number;
  gridVisible: boolean;
  snapToGrid: boolean;
  projectLocked: boolean;
  selectedNodeIds: string[];
  selectedDrawingId: string | null;
  onCenterChange: (x: number, y: number) => void;
  selection: Selection;
  connecting: boolean;
  linkEndpoints: LinkEndpoint[];
  onNodeClick: (id: string) => void;
  onMultiSelectNode: (id: string) => void;
  onMultiSelect: (ids: string[]) => void;
  onLinkClick: (id: string) => void;
  onCanvasClick: (point: { x: number; y: number }) => void;
  onPortClick: (endpoint: LinkEndpoint) => void;
  onNodeContext: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
  onNodeLabelMove: (id: string, x: number, y: number) => void;
  onLinkEndpointLabelMove: (linkId: string, nodeId: string, x: number, y: number) => void;
  onDrawingClick: (id: string) => void;
  onDrawingMove: (id: string, x: number, y: number) => void;
  onDeleteSelected: () => void;
  onClearSelection: () => void;
  onZoom: (delta: number) => void;
  onDropTemplate: (templateId: string, x: number, y: number) => void;
};

type DragState = { nodeIds: string[]; pointerId: number; start: { x: number; y: number }; origins: Map<string, { x: number; y: number }> };
type LabelDragState = { nodeId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type LinkLabelDragState = { linkId: string; nodeId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type DrawingDragState = { drawingId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type PanState = { pointerId: number; startX: number; startY: number; centerX: number; centerY: number };

function TopologyCanvas(props: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const labelDragRef = useRef<LabelDragState | null>(null);
  const linkLabelDragRef = useRef<LinkLabelDragState | null>(null);
  const drawingDragRef = useRef<DrawingDragState | null>(null);
  const selectionRef = useRef<{ pointerId: number; start: { x: number; y: number }; additive: boolean } | null>(null);
  const panRef = useRef<PanState | null>(null);
  const spaceDownRef = useRef(false);
  const panMovedRef = useRef(false);
  const movedRef = useRef(false);
  const lastMoveRef = useRef<Map<string, { x: number; y: number }> | null>(null);
  const [dropActive, setDropActive] = useState(false);
  const [selectionBox, setSelectionBox] = useState<{ x: number; y: number; width: number; height: number } | null>(null);
  const [drawingPositions, setDrawingPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [labelPositions, setLabelPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [linkLabelPositions, setLinkLabelPositions] = useState<Record<string, { x: number; y: number }>>({});
  const view = { width: scene.width / props.zoom, height: scene.height / props.zoom, x: props.centerX - scene.width / props.zoom / 2, y: props.centerY - scene.height / props.zoom / 2 };
  const nodeMap = useMemo(() => new Map(props.nodes.map((node) => [node.node_id, node])), [props.nodes]);
  const renderedDrawings = useMemo(() => props.drawings.flatMap((drawing) => {
    const markup = typeof drawing.svg === "string" ? sanitizeDrawingSvg(drawing.svg) : "";
    return markup ? [{ drawing, markup }] : [];
  }), [props.drawings]);
  const orderedDrawings = useMemo(() => [...renderedDrawings].sort((left, right) => Number(left.drawing.z_index ?? 0) - Number(right.drawing.z_index ?? 0)), [renderedDrawings]);

  const scenePoint = (clientX: number, clientY: number, rect: DOMRect) => {
    const scale = Math.min(rect.width / view.width, rect.height / view.height);
    const offsetX = (rect.width - view.width * scale) / 2;
    const offsetY = (rect.height - view.height * scale) / 2;
    const x = view.x + (clientX - rect.left - offsetX) / scale;
    const y = view.y + (clientY - rect.top - offsetY) / scale;
    return props.snapToGrid ? { x: Math.round(x / 10) * 10, y: Math.round(y / 10) * 10 } : { x: Math.round(x), y: Math.round(y) };
  };

  function startMove(event: PointerEvent<SVGGElement>, node: Node) {
    if (props.connecting || props.projectLocked || node.locked || event.button !== 0 || (event.target as Element).closest("[data-interface-port], [data-node-label]")) return;
    if (event.metaKey || event.ctrlKey || !svgRef.current) return;
    event.stopPropagation();
    const nodeIds = props.selectedNodeIds.includes(node.node_id) ? props.selectedNodeIds : [node.node_id];
    const origins = new Map(nodeIds.map((id) => { const item = nodeMap.get(id); return [id, { x: Number(item?.x ?? 0), y: Number(item?.y ?? 0) }] as const; }));
    dragRef.current = { nodeIds, pointerId: event.pointerId, start: scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect()), origins };
    if (!props.selectedNodeIds.includes(node.node_id)) props.onMultiSelect(nodeIds);
    movedRef.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startLabelMove(event: PointerEvent<SVGTextElement>, node: Node) {
    if (props.projectLocked || node.locked || event.button !== 0 || props.connecting || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
    labelDragRef.current = { nodeId: node.node_id, pointerId: event.pointerId, start: point, origin: { x: Number(label.x ?? 0), y: Number(label.y ?? 52) } };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startLinkLabelMove(event: PointerEvent<SVGTextElement>, link: Link, endpoint: LinkEndpoint, node: Node) {
    if (props.projectLocked || node.locked || event.button !== 0 || props.connecting || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    const label = endpoint.label;
    const port = localPortPoint(portIndex(node, endpoint), portCount(node));
    linkLabelDragRef.current = {
      linkId: link.link_id,
      nodeId: endpoint.node_id,
      pointerId: event.pointerId,
      start: point,
      origin: { x: Number(label?.x ?? port.x + 8), y: Number(label?.y ?? port.y + 4) },
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startDrawingMove(event: PointerEvent<SVGGElement>, drawing: Drawing) {
    if (props.projectLocked || event.button !== 0 || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    drawingDragRef.current = { drawingId: drawing.drawing_id, pointerId: event.pointerId, start: point, origin: { x: Number(drawing.x ?? 0), y: Number(drawing.y ?? 0) } };
    props.onDrawingClick(drawing.drawing_id);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startPan(event: PointerEvent<SVGSVGElement>) {
    const isBackground = event.target === event.currentTarget || (event.target as Element).tagName === "rect";
    const panButton = event.button === 1 || (event.button === 0 && spaceDownRef.current);
    if (!isBackground || (event.button !== 0 && event.button !== 1) || !svgRef.current) return;
    if (panButton) {
      event.preventDefault();
      panRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, centerX: props.centerX, centerY: props.centerY };
      panMovedRef.current = false;
      svgRef.current.setPointerCapture(event.pointerId);
      return;
    }
    const start = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    selectionRef.current = { pointerId: event.pointerId, start, additive: event.shiftKey || event.metaKey || event.ctrlKey };
    setSelectionBox({ x: start.x, y: start.y, width: 0, height: 0 });
    panMovedRef.current = false;
    svgRef.current.setPointerCapture(event.pointerId);
  }

  function moveNode(event: PointerEvent<SVGSVGElement>) {
    const selection = selectionRef.current;
    if (selection && selection.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      if (Math.abs(point.x - selection.start.x) + Math.abs(point.y - selection.start.y) > 2) panMovedRef.current = true;
      setSelectionBox({ x: Math.min(selection.start.x, point.x), y: Math.min(selection.start.y, point.y), width: Math.abs(point.x - selection.start.x), height: Math.abs(point.y - selection.start.y) });
      return;
    }
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
    const linkLabelDrag = linkLabelDragRef.current;
    if (linkLabelDrag && linkLabelDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      const key = `${linkLabelDrag.linkId}:${linkLabelDrag.nodeId}`;
      setLinkLabelPositions((current) => ({ ...current, [key]: { x: linkLabelDrag.origin.x + point.x - linkLabelDrag.start.x, y: linkLabelDrag.origin.y + point.y - linkLabelDrag.start.y } }));
      return;
    }
    const labelDrag = labelDragRef.current;
    if (labelDrag && labelDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      setLabelPositions((current) => ({ ...current, [labelDrag.nodeId]: { x: labelDrag.origin.x + point.x - labelDrag.start.x, y: labelDrag.origin.y + point.y - labelDrag.start.y } }));
      return;
    }
    const drawingDrag = drawingDragRef.current;
    if (drawingDrag && drawingDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      setDrawingPositions((current) => ({ ...current, [drawingDrag.drawingId]: { x: drawingDrag.origin.x + point.x - drawingDrag.start.x, y: drawingDrag.origin.y + point.y - drawingDrag.start.y } }));
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId || !svgRef.current) return;
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    movedRef.current = true;
    const delta = { x: point.x - drag.start.x, y: point.y - drag.start.y };
    drag.nodeIds.forEach((id) => { const origin = drag.origins.get(id); if (origin) props.onNodeMove(id, origin.x + delta.x, origin.y + delta.y); });
  }

  function endMove(event: PointerEvent<SVGSVGElement>) {
    const selection = selectionRef.current;
    if (selection && selection.pointerId === event.pointerId && svgRef.current) {
      selectionRef.current = null;
      const end = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      const ids = selectionBoxIds(props.nodes, selection.start, end);
      props.onMultiSelect(selection.additive ? Array.from(new Set([...props.selectedNodeIds, ...ids])) : ids);
      if (ids.length === 1) props.onNodeClick(ids[0]!);
      setSelectionBox(null);
      return;
    }
    if (panRef.current?.pointerId === event.pointerId) {
      panRef.current = null;
      return;
    }
    const linkLabelDrag = linkLabelDragRef.current;
    if (linkLabelDrag?.pointerId === event.pointerId) {
      linkLabelDragRef.current = null;
      const key = `${linkLabelDrag.linkId}:${linkLabelDrag.nodeId}`;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        props.onLinkEndpointLabelMove(
          linkLabelDrag.linkId,
          linkLabelDrag.nodeId,
          linkLabelDrag.origin.x + point.x - linkLabelDrag.start.x,
          linkLabelDrag.origin.y + point.y - linkLabelDrag.start.y,
        );
      }
      setLinkLabelPositions((current) => { const next = { ...current }; delete next[key]; return next; });
      movedRef.current = false;
      return;
    }
    const labelDrag = labelDragRef.current;
    if (labelDrag?.pointerId === event.pointerId) {
      labelDragRef.current = null;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        props.onNodeLabelMove(labelDrag.nodeId, labelDrag.origin.x + point.x - labelDrag.start.x, labelDrag.origin.y + point.y - labelDrag.start.y);
      }
      setLabelPositions((current) => { const next = { ...current }; delete next[labelDrag.nodeId]; return next; });
      movedRef.current = false;
      return;
    }
    const drawingDrag = drawingDragRef.current;
      if (drawingDrag?.pointerId === event.pointerId) {
      drawingDragRef.current = null;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        const x = drawingDrag.origin.x + point.x - drawingDrag.start.x;
        const y = drawingDrag.origin.y + point.y - drawingDrag.start.y;
        setDrawingPositions((current) => { const next = { ...current }; delete next[drawingDrag.drawingId]; return next; });
        props.onDrawingMove(drawingDrag.drawingId, x, y);
      }
      movedRef.current = false;
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (movedRef.current && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      const delta = { x: point.x - drag.start.x, y: point.y - drag.start.y };
      lastMoveRef.current = drag.origins;
      drag.nodeIds.forEach((id) => { const origin = drag.origins.get(id); if (origin) props.onNodeMoveEnd(id, origin.x + delta.x, origin.y + delta.y); });
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
    <svg ref={svgRef} viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} role="group" aria-label="Network topology canvas" tabIndex={0} onPointerDown={startPan} onPointerMove={moveNode} onPointerUp={endMove} onPointerCancel={endMove} onKeyDown={(event) => {
      const mod = event.metaKey || event.ctrlKey;
      if (event.code === "Space") { event.preventDefault(); spaceDownRef.current = true; }
      if (mod && event.key.toLowerCase() === "a") { event.preventDefault(); props.onMultiSelect(props.nodes.map((node) => node.node_id)); }
      if (mod && (event.key === "+" || event.key === "=")) { event.preventDefault(); props.onZoom(0.1); }
      if (mod && event.key === "-") { event.preventDefault(); props.onZoom(-0.1); }
      if (mod && event.key.toLowerCase() === "z" && lastMoveRef.current) { event.preventDefault(); lastMoveRef.current.forEach((point, id) => props.onNodeMoveEnd(id, point.x, point.y)); lastMoveRef.current = null; }
      if (event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); props.onDeleteSelected(); }
      if (event.key === "Escape") props.onClearSelection();
    }} onKeyUp={(event) => { if (event.code === "Space") spaceDownRef.current = false; }} onBlur={() => { spaceDownRef.current = false; }} onClick={(event) => {
      if (panMovedRef.current) { panMovedRef.current = false; return; }
      if (event.target === event.currentTarget || (event.target as Element).tagName === "rect") {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current!.getBoundingClientRect());
        props.onCanvasClick(point);
      }
    }} onContextMenu={(event) => { if ((event.target as Element).tagName === "rect") props.onClearSelection(); }}>
      <defs><pattern id="sim-grid" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="currentColor" /></pattern></defs>
      <rect className={`sim-grid-fill ${props.gridVisible ? "" : "is-hidden"}`} x={view.x} y={view.y} width={view.width} height={view.height} fill="url(#sim-grid)" />
      {orderedDrawings.map(({ drawing, markup }) => {
        const position = drawingPositions[drawing.drawing_id] ?? { x: Number(drawing.x ?? 0), y: Number(drawing.y ?? 0) };
        const selected = props.selectedDrawingId === drawing.drawing_id;
        const width = Number(drawing.width ?? 160); const height = Number(drawing.height ?? 90);
        return <g key={drawing.drawing_id} className={`sim-drawing ${selected ? "is-selected" : ""}`} transform={`translate(${position.x},${position.y}) rotate(${Number(drawing.rotation ?? 0)})`} onPointerDown={(event) => startDrawingMove(event, drawing)} onClick={(event) => { event.stopPropagation(); props.onDrawingClick(drawing.drawing_id); }} onContextMenu={() => props.onDrawingClick(drawing.drawing_id)}>
          {selected && <rect className="sim-drawing-selection" x="-3" y="-3" width={width + 6} height={height + 6} />}
          <g dangerouslySetInnerHTML={{ __html: markup }} />
        </g>;
      })}
      {props.links.map((link) => {
        const endpoints = (link.nodes ?? []).map((endpoint) => ({ endpoint, node: nodeMap.get(endpoint.node_id) })).filter((item): item is { endpoint: LinkEndpoint; node: Node } => Boolean(item.node));
        const source = endpoints[0];
        if (!source || endpoints.length < 2) return null;
        const selected = props.selection?.kind === "link" && props.selection.id === link.link_id;
        const sourcePoint = portPoint(source.node, source.endpoint);
        const style = (link.link_style ?? {}) as Record<string, unknown>;
        return <g key={link.link_id} className={`sim-link ${selected ? "is-selected" : ""} ${link.suspend ? "is-suspended" : ""}`} onClick={(event) => { event.stopPropagation(); props.onLinkClick(link.link_id); }} onContextMenu={() => props.onLinkClick(link.link_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") props.onLinkClick(link.link_id); }} role="button" aria-label={`Ethernet segment connecting ${endpoints.map(({ node }) => node.name).join(", ")}`} tabIndex={0}>
          {endpoints.slice(1).map(({ endpoint, node }) => { const point = portPoint(node, endpoint); return <g key={`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`}><line x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} style={{ stroke: String(style.color ?? ""), strokeWidth: Number(style.width ?? 0) || undefined, strokeDasharray: link.suspend ? "6 4" : undefined }} /><line className="sim-link-hit" x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} /></g>; })}
          {endpoints.map(({ endpoint, node }) => {
            const localPort = localPortPoint(portIndex(node, endpoint), portCount(node));
            const key = `${link.link_id}:${endpoint.node_id}`;
            const savedLabel = endpoint.label;
            const text = savedLabel ? String(savedLabel.text ?? "") : portLabel(node, endpoint);
            if (!text) return null;
            const position = linkLabelPositions[key] ?? { x: Number(savedLabel?.x ?? localPort.x + 8), y: Number(savedLabel?.y ?? localPort.y + 4) };
            return <g key={`label:${endpoint.node_id}`} transform={`translate(${Number(node.x ?? 0)},${Number(node.y ?? 0)})`}>
              <text className="sim-link-label" x={position.x} y={position.y} transform={Number(savedLabel?.rotation) ? `rotate(${Number(savedLabel?.rotation)}, ${position.x}, ${position.y})` : undefined} onPointerDown={(event) => startLinkLabelMove(event, link, endpoint, node)} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.stopPropagation()}>{text}</text>
            </g>;
          })}
        </g>;
      })}
      {props.nodes.map((node) => {
        const x = Number(node.x ?? 0);
        const y = Number(node.y ?? 0);
        const selected = (props.selection?.kind === "node" && props.selection.id === node.node_id) || props.selectedNodeIds.includes(node.node_id);
        const symbol = resolveCanvasSymbol(node.node_type ?? "", node.symbol, props.symbolDefaults, props.customSymbolIds);
        const Icon = symbol.kind === "default" ? iconForSymbolName(symbol.name, node.node_type ?? "") : iconForType(node.node_type ?? "");
        const endpoints = nodePorts(node);
        const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
        const labelPosition = labelPositions[node.node_id] ?? { x: Number(label.x ?? 0), y: Number(label.y ?? 52) };
        return <g key={node.node_id} transform={`translate(${x},${y})`} className={`sim-node ${selected ? "is-selected" : ""} ${props.connecting ? "is-connecting" : ""} ${node.locked || props.projectLocked ? "is-locked" : ""}`} onPointerDown={(event) => startMove(event, node)} onClick={(event) => { event.stopPropagation(); if (event.metaKey || event.ctrlKey) props.onMultiSelectNode(node.node_id); else if (!movedRef.current) props.onNodeClick(node.node_id); movedRef.current = false; }} onContextMenu={() => props.onNodeContext(node.node_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onNodeClick(node.node_id); } }} role="button" aria-label={`Node ${node.name}`} tabIndex={0}>
          <rect className="sim-node-card" x="-56" y="-36" width="112" height="66" rx="9" />
          <Icon className="sim-node-icon" x={-13} y={-29} width={26} height={26} strokeWidth={1.7} />
          {symbol.kind === "custom" && <AuthenticatedSymbolImage symbolId={symbol.symbolId} />}
          <text className="sim-node-type" y="8">{kindLabel(node.node_type ?? "device")}</text>
          <circle className={`sim-node-status ${node.status === "started" ? "running" : "stopped"}`} cx="46" cy="-26" r="4" />
          <text className="sim-node-name" data-node-label="true" x={labelPosition.x} y={labelPosition.y} onPointerDown={(event) => startLabelMove(event, node)} onClick={(event) => event.stopPropagation()}>{node.name}</text>
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
      {selectionBox && <rect className="sim-selection-box" x={selectionBox.x} y={selectionBox.y} width={selectionBox.width} height={selectionBox.height} />}
      {props.nodes.length === 0 && <g className="sim-canvas-empty"><Network x={-16} y={-30} width={32} height={32} /><text y="54">Your canvas is ready</text><text className="sim-empty-subtitle" y="77">Drop a device here or add one from the library on the left.</text></g>}
    </svg>
  </div>;
}

function AuthenticatedSymbolImage({ symbolId }: { symbolId: string }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    setUrl(null);
    void api.symbolRaw(symbolId).then((blob) => {
      objectUrl = URL.createObjectURL(blob);
      if (cancelled) URL.revokeObjectURL(objectUrl);
      else setUrl(objectUrl);
    }).catch(() => {});
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [symbolId]);
  return url
    ? <image className="sim-node-symbol" href={url} x="-23" y="-32" width="46" height="38" preserveAspectRatio="xMidYMid meet" />
    : null;
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
  if (node?.node_type === "qemu") return `Ethernet${endpoint.adapter_number}`;
  if (node?.node_type === "dynamips") return `Ethernet${endpoint.port_number}`;
  if (["ethernet_switch", "ethernet_hub", "cloud", "nat"].includes(node?.node_type ?? "")) {
    const ports = node?.ports_mapping;
    const port = Array.isArray(ports) ? ports[endpoint.port_number] as { name?: unknown } | undefined : undefined;
    return typeof port?.name === "string" ? port.name : `Ethernet${endpoint.port_number}`;
  }
  return `eth${endpoint.port_number}`;
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
  if (type === "ethernet_switch" || type === "ethernet_hub" || type === "nat") return Network;
  if (type === "cloud") return Cloud;
  return HardDrive;
}

function iconForSymbolName(name: string, type: string) {
  const key = name.toLocaleLowerCase();
  if (key.includes("router")) return Router;
  if (key.includes("switch") || key.includes("hub")) return Network;
  if (key.includes("cloud")) return Cloud;
  if (key === "pc" || key.includes("computer")) return Monitor;
  return iconForType(type);
}

function kindLabel(kind: string) {
  return ({ dynamips: "Cisco IOS", qemu: "QEMU / KVM", docker: "Docker", vpcs: "VPCS", ethernet_switch: "Ethernet switch", ethernet_hub: "Ethernet hub", cloud: "Cloud", nat: "NAT", device: "Device" } as Record<string, string>)[kind] ?? kind;
}

function deviceGroupLabel(kind: string) {
  return kind === "dynamips" ? "Dynamips · Cisco IOS" : kindLabel(kind);
}

function templateDescription(template: RuntimeTemplate) {
  if (template.template_type === "qemu") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · ${template.memory} MB RAM`;
  if (template.template_type === "docker") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · Docker image`;
  if (template.template_type === "dynamips") return `${template.platform} · Cisco IOS`;
  if (template.template_type === "ethernet_switch") return `${template.ports_mapping.length} Ethernet ports · VLAN switching`;
  if (template.template_type === "ethernet_hub") return `${template.ports_mapping.length} Ethernet ports · shared medium`;
  if (template.template_type === "cloud") return `${template.ports_mapping.length} host ports · Ethernet, TAP, or UDP`;
  if (template.template_type === "nat") return "Host NAT network · libvirt virbr0";
  return "Lightweight virtual PC";
}

function imageName(node: Node) {
  const image = typeof node.image === "string" ? node.image : typeof node.hda_disk_image === "string" ? node.hda_disk_image : "";
  return image.split("/").at(-1) || kindLabel(node.node_type ?? "device");
}

function valuesFromDrawing(drawing: Drawing): DrawingValues {
  const source = typeof drawing.svg === "string" ? drawing.svg : "";
  const parsed = new DOMParser().parseFromString(source, "image/svg+xml");
  const svg = parsed.documentElement;
  const shape = svg.firstElementChild;
  const getNumber = (name: string, fallback: number) => Number(svg.getAttribute(name) ?? drawing[name] ?? fallback) || fallback;
  const fill = shape?.getAttribute("fill") ?? "#dbeafe";
  return {
    width: getNumber("width", 180), height: getNumber("height", 100),
    stroke: shape?.getAttribute("stroke") ?? "#2563eb",
    fill: fill === "none" ? "#ffffff" : fill,
    strokeWidth: Number(shape?.getAttribute("stroke-width") ?? 2) || 2,
    text: shape?.textContent ?? "Text",
    ...(shape?.localName === "image" && shape.getAttribute("href") ? { imageDataUrl: shape.getAttribute("href")! } : {}),
  };
}

function safeFileName(value: string) {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "network-map";
}

function messageFor(reason: unknown) {
  return reason instanceof Error ? reason.message : "The controller did not complete this request.";
}

function nodeHasAuxConsole(node: Node) {
  const properties = node.properties && typeof node.properties === "object" ? node.properties as Record<string, unknown> : {};
  return String(node.aux_type ?? properties.aux_type ?? "none") !== "none" && Number(properties.aux ?? node.aux_console_port ?? 0) > 0;
}
