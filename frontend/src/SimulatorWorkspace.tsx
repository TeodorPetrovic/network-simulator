import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Link, LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { api, getAuthToken } from "./api";
import { ResourceMonitorPopover } from "./ResourceMonitorPopover";
import { nodesForLifecycle } from "./lifecycle";
import type { LifecycleAction } from "./lifecycle";
import { Card } from "@/components/ui/card";
import {
  ContextMenu, ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { NodeSettingsDialog } from "./NodeSettingsDialog";
import { DrawingEditorDialog, HelpDialog, InfoDialog, LinkPropertiesDialog, LogConsoleDialog, ScreenshotDialog, SnapshotsDialog, SymbolPickerDialog } from "./map/MapDialogs";
import type { ScreenshotRequest } from "./map/MapDialogs";
import { serializeSvgScreenshot, svgScreenshotPng } from "./map/helpers";
import type { DrawingTool } from "./map/helpers";
import { TopologyCanvas, scene, maxZoom, messageFor, safeFileName, sameEndpoint, valuesFromDrawing } from "./map/TopologyCanvas";
import { pngScreenshotAvailable, spreadOverlappingNodes } from "./workspaceUtils";
import type { Selection } from "./map/TopologyCanvas";
import { alignNodes, distributeNodes } from "./map/layout";

import { WorkspaceSidebar } from "./WorkspaceSidebar";
import { WorkspaceDocks } from "./WorkspaceDocks";
import { WorkspaceContextMenu } from "./WorkspaceContextMenu";
import { WorkspaceIconToolbar, WorkspaceToolbar } from "./WorkspaceToolbar";
import { ProjectStatusBar } from "./ProjectStatusBar";
import { useWorkspaceSelection } from "./useWorkspaceSelection";
import { useWorkspaceActions } from "./useWorkspaceActions";
import { CaptureWindow } from "./capture/CaptureWindow";
import { WebWiresharkWindow } from "./capture/WebWiresharkWindow";
import { webWiresharkClientUrl } from "./api";

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
  onApiError?: (message: string) => void;
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
  onNodeMoveEnd: (positions: Array<{ id: string; x: number; y: number }>) => void;
  consoleOpen?: boolean;
  onCloseConsoleDock?: () => void;
  onOpenConsoleDock?: () => void;
  onToggleConsoleDock?: () => void;
};


export function SimulatorWorkspace(props: SimulatorWorkspaceProps) {
  const [captureViewer, setCaptureViewer] = useState<{ url?: string; downloadUrl?: string; title: string } | null>(null);
  const [wiresharkWindows, setWiresharkWindows] = useState<Array<{ linkId: string; wsPath: string; title: string; key: number }>>([]);
  const openedEvents = useMemo(() => new Set<string>(), [props.project.project_id]);
  useEffect(() => {
    for (const link of props.topology.links) {
      const path = link.wireshark_ws_path;
      if (link.wireshark === true && typeof path === "string" && !openedEvents.has(link.link_id)) {
        openedEvents.add(link.link_id);
        setWiresharkWindows((current) => current.some((item) => item.linkId === link.link_id) ? current : [...current, { linkId: link.link_id, wsPath: path, title: `Web Wireshark · ${link.capture_file_name ?? link.link_id}`, key: Date.now() }]);
      }
      if (link.capturing === false || link.wireshark === false) setWiresharkWindows((current) => current.filter((item) => item.linkId !== link.link_id));
    }
  }, [props.topology.links, openedEvents]);
  function openWebWireshark(link: Link, newTab = false) {
    const wsPath = link.wireshark_ws_path ?? link.ws_path;
    if (typeof wsPath !== "string") return;
    if (newTab) { window.open(webWiresharkClientUrl(wsPath), "_blank", "noopener"); return; }
    setWiresharkWindows((current) => current.some((item) => item.linkId === link.link_id) ? current : [...current, { linkId: link.link_id, wsPath, title: `Web Wireshark · ${link.capture_file_name ?? link.link_id}`, key: Date.now() }]);
  }
  async function restartWebWireshark(link: Link) {
    try { const updated = await api.restartWebWireshark(props.project.project_id, link.link_id); const wsPath = updated.wireshark_ws_path ?? updated.ws_path ?? link.wireshark_ws_path ?? link.ws_path; if (typeof wsPath === "string") setWiresharkWindows((current) => current.map((item) => item.linkId === link.link_id ? { ...item, wsPath, key: Date.now() } : item)); }
    catch (reason) { setFeatureMessage(reason instanceof Error ? reason.message : "Could not restart Web Wireshark."); }
  }
  async function stopLinkCapture(link: Link) {
    try { await api.stopLinkCapture(props.project.project_id, link.link_id); setWiresharkWindows((current) => current.filter((item) => item.linkId !== link.link_id)); setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === link.link_id ? { ...item, capturing: false, wireshark: false } : item) })); }
    catch (reason) { setFeatureMessage(reason instanceof Error ? reason.message : "Could not stop packet capture."); }
  }
  async function resetLink(link: Link) {
    try {
      const updated = await api.updateLink(props.project.project_id, link.link_id, { filters: {}, suspend: false });
      setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === link.link_id ? updated : item) }));
      setFeatureMessage("Link reset: filters cleared and link resumed.");
    } catch (reason) { const message = `Could not reset link: ${reason instanceof Error ? reason.message : "request failed"}`; setFeatureMessage(message); props.onApiError?.(message); }
  }
  async function openCaptureViewer(link: Link, newTab = false) {
    const capturing = Boolean(link.capturing);
    const route = capturing
      ? api.linkCaptureStreamUrl(props.project.project_id, link.link_id)
      : api.linkCaptureFileUrl(props.project.project_id, link.link_id);
    const url = new URL(route, window.location.href);
    if (getAuthToken()) url.searchParams.set("ticket", await api.accessTicket(route));
    const title = String(link.capture_file_name ?? link.link_id);
    const viewer = capturing
      ? { url: url.toString(), title: `Capture · ${title}` }
      : { downloadUrl: url.toString(), title: `Capture · ${title}` };
    if (newTab) {
      const params = new URLSearchParams({ title });
      if (viewer.url) params.set("stream", viewer.url);
      if (viewer.downloadUrl) params.set("download", viewer.downloadUrl);
      window.open(`/capture?${params.toString()}`, "_blank", "noopener");
      return;
    }
    setCaptureViewer(viewer);
  }
  const [view, setView] = useState({ zoom: 1, centerX: 0, centerY: 0 });
  const viewKey = `ns.view.${props.project.project_id}`;
  const [savedViewLoaded, setSavedViewLoaded] = useState(false);
  useEffect(() => {
    setSavedViewLoaded(false);
    try {
      const raw = localStorage.getItem(`ns.view.${props.project.project_id}`);
      const saved = raw ? JSON.parse(raw) as { zoom?: number; centerX?: number; centerY?: number } : null;
      if (saved && Number.isFinite(saved.zoom) && Number.isFinite(saved.centerX) && Number.isFinite(saved.centerY)) {
        setView({ zoom: Math.max(0.1, Math.min(maxZoom, saved.zoom!)), centerX: saved.centerX!, centerY: saved.centerY! });
      }
    } catch { /* Ignore invalid or unavailable local storage. */ }
    setSavedViewLoaded(true);
  }, [props.project.project_id]);
  useEffect(() => { if (savedViewLoaded) try { localStorage.setItem(viewKey, JSON.stringify(view)); } catch { /* Storage can be disabled. */ } }, [savedViewLoaded, view, viewKey]);
  const [showInterfaceLabels, setShowInterfaceLabels] = useState(false);
  const [topology, setTopology] = useState(props.topology);
  const [deviceQuery, setDeviceQuery] = useState("");
  const [dockState, setDockState] = useState(() => { try { const saved = JSON.parse(localStorage.getItem("ns.docks") ?? "{}"); return { topology: saved.topology ?? true, servers: saved.servers ?? true }; } catch { return { topology: true, servers: true }; } });
  const topologyDockOpen = dockState.topology;
  const serversDockOpen = dockState.servers;
  const setTopologyDockOpen = (open: boolean) => setDockState((current) => ({ ...current, topology: open }));
  const setServersDockOpen = (open: boolean) => setDockState((current) => ({ ...current, servers: open }));
  useEffect(() => { try { localStorage.setItem("ns.docks", JSON.stringify(dockState)); } catch { /* Storage can be disabled. */ } }, [dockState]);
  const [configureNode, setConfigureNode] = useState<Node | null>(null);
  const [snapshotsOpen, setSnapshotsOpen] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const [logCounts, setLogCounts] = useState({ errors: 0, warnings: 0 });
  const [screenshotOpen, setScreenshotOpen] = useState(false);
  const [linkPropertiesOpen, setLinkPropertiesOpen] = useState(false);
  const [symbolNode, setSymbolNode] = useState<Node | null>(null);
  const [drawingTool, setDrawingTool] = useState<DrawingTool | null>(null);
  const [pendingDrawing, setPendingDrawing] = useState<{ tool: DrawingTool; values: DrawingValues } | null>(null);
  const [drawingEditOpen, setDrawingEditOpen] = useState(false);
  const [drawingEditTool, setDrawingEditTool] = useState<DrawingTool | null>(null);
  const [drawingEditValues, setDrawingEditValues] = useState<DrawingValues | null>(null);
  const [gridVisible, setGridVisible] = useState(false);
  const [snapToGrid, setSnapToGrid] = useState(() => window.localStorage.getItem("network-simulator.snapToGrid") === "true");
  useEffect(() => { window.localStorage.setItem("network-simulator.snapToGrid", String(snapToGrid)); }, [snapToGrid]);
  const [projectLocked, setProjectLocked] = useState(Boolean(props.project.locked));
  const [symbolDefaults, setSymbolDefaults] = useState<Record<string, string>>({});
  const [customSymbolIds, setCustomSymbolIds] = useState<string[]>([]);
  const copiedNodes = useRef<{ nodes: Node[]; links: Link[] } | null>(null);
  const [featureMessage, setFeatureMessage] = useState("");
  const groupedTemplates = useMemo(() => {
    const query = deviceQuery.trim().toLowerCase();
    return ["dynamips", "qemu", "docker", "vpcs", "ethernet_switch", "ethernet_hub", "cloud", "nat"].map((kind) => ({
      kind,
      templates: props.templates.filter((template) => template.template_type === kind && (!query || `${template.name} ${template.template_type}`.toLowerCase().includes(query))),
    }));
  }, [deviceQuery, props.templates]);
  const canvasNodes = useMemo(() => spreadOverlappingNodes(topology.nodes), [topology.nodes]);
  const { selectedNodeIds, setSelectedNodeIds, selectedDrawingId, setSelectedDrawingId, selectedDrawing } = useWorkspaceSelection(props.selection, topology);
  const selectedNode = topology.nodes.find((node) => node.node_id === (props.selection?.kind === "node" ? props.selection.id : selectedNodeIds[0])) ?? props.selectedNode;
  const selectedLink = props.selection?.kind === "link" ? topology.links.find((link) => link.link_id === props.selection?.id) ?? props.selectedLink : props.selectedLink;
  const { placeDrawing, saveEditedDrawing, removeDrawing, moveDrawing, changeDrawingOrder, toggleProjectLock, toggleNodeLock, toggleNodeIsolation, toggleLinkSuspend, saveNodeLabelPosition, saveLinkEndpointLabelPosition } = useWorkspaceActions({ workspace: props, topology, selectedDrawing, pendingDrawing, projectLocked, setTopology, setSelectedNodeIds, setSelectedDrawingId, setPendingDrawing, setFeatureMessage, setDrawingEditOpen, setProjectLocked });
  const selectedLifecycleNodes = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && !node.locked);
  const onLogCount = useCallback((level: "error" | "warning") => setLogCounts((current) => ({ ...current, [level === "error" ? "errors" : "warnings"]: current[level === "error" ? "errors" : "warnings"] + 1 })), []);

  useEffect(() => { setTopology(props.topology); }, [props.topology]);
  useEffect(() => {
    if (!props.connecting) return;
    const cancel = (event: KeyboardEvent) => { if (event.key === "Escape") { props.onToggleConnect(); setFeatureMessage("Link drawing cancelled."); } };
    window.addEventListener("keydown", cancel);
    return () => window.removeEventListener("keydown", cancel);
  }, [props.connecting, props.onToggleConnect]);
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
    const zoom = Math.max(0.1, Math.min(maxZoom, Math.min(scene.width / (maxX - minX + 244), scene.height / (maxY - minY + 166))));
    setView({ zoom, centerX: (minX + maxX) / 2, centerY: (minY + maxY) / 2 });
  }

  function zoomToSelection() {
    const selectedIds = new Set(selectedNodeIds);
    if (props.selection?.kind === "node") selectedIds.add(props.selection.id);
    if (props.selection?.kind === "link") topology.links.find((item) => item.link_id === props.selection?.id)?.nodes.forEach((endpoint) => selectedIds.add(endpoint.node_id));
    const boxes = canvasNodes.filter((node) => selectedIds.has(node.node_id)).map((node) => ({ left: Number(node.x ?? 0) - 56, right: Number(node.x ?? 0) + 56, top: Number(node.y ?? 0) - 36, bottom: Number(node.y ?? 0) + 36 }));
    if (selectedDrawing) { const x = Number(selectedDrawing.x ?? 0); const y = Number(selectedDrawing.y ?? 0); boxes.push({ left: x, right: x + Number(selectedDrawing.width ?? 160), top: y, bottom: y + Number(selectedDrawing.height ?? 90) }); }
    if (!boxes.length) { setFeatureMessage("Select a device, link, or drawing to zoom to it."); return; }
    const left = Math.min(...boxes.map((box) => box.left)); const right = Math.max(...boxes.map((box) => box.right));
    const top = Math.min(...boxes.map((box) => box.top)); const bottom = Math.max(...boxes.map((box) => box.bottom));
    const zoom = Math.max(0.1, Math.min(maxZoom, Math.min(scene.width / (right - left + 80), scene.height / (bottom - top + 80))));
    setView({ zoom, centerX: (left + right) / 2, centerY: (top + bottom) / 2 });
  }

  useEffect(() => {
    if (savedViewLoaded && !localStorage.getItem(viewKey)) fitTopology();
  }, [props.project.project_id, savedViewLoaded]);

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

  function runAllLifecycle(action: LifecycleAction) {
    const targets = nodesForLifecycle(topology.nodes, action);
    targets.forEach((node) => props.onNodeLifecycle(node.node_id, action));
    if (targets.length) setFeatureMessage(`${action[0]?.toUpperCase()}${action.slice(1)} requested for ${targets.length} device${targets.length === 1 ? "" : "s"}.`);
  }

  function deleteSelectedNodes() {
    const targets = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id));
    if (targets.length === 1 && props.selection?.kind === "node") { props.onDeleteSelection(); return; }
    if (targets.length && !window.confirm(`Delete ${targets.length} selected device${targets.length === 1 ? "" : "s"}?`)) return;
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

  function arrangeSelection(mode: "left" | "right" | "top" | "bottom" | "horizontal" | "vertical" | "distribute-horizontal" | "distribute-vertical") {
    if (projectLocked) return;
    const selected = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && !node.locked);
    const positions = mode.startsWith("distribute-")
      ? distributeNodes(selected, mode.endsWith("horizontal") ? "horizontal" : "vertical")
      : alignNodes(selected, mode as "left" | "right" | "top" | "bottom" | "horizontal" | "vertical");
    if (positions.length > 1) props.onNodeMoveEnd(positions);
  }

  function copySelectedNodes() {
    const ids = new Set(selectedNodeIds);
    const nodes = topology.nodes.filter((node) => ids.has(node.node_id));
    if (!nodes.length) return;
    copiedNodes.current = { nodes, links: topology.links.filter((link) => link.nodes.length >= 2 && link.nodes.every((endpoint) => ids.has(endpoint.node_id))) };
    setFeatureMessage(`${nodes.length} device${nodes.length === 1 ? "" : "s"} copied.`);
  }

  async function pasteCopiedNodes() {
    const copied = copiedNodes.current;
    if (!copied || projectLocked) return;
    const idMap = new Map<string, string>();
    const created: Node[] = [];
    const createdLinks: Link[] = [];
    try {
      for (const source of copied.nodes) {
        const payload: Record<string, unknown> = { ...source };
        for (const key of ["node_id", "project_id", "container_id", "image_digest", "status", "locked", "runtime_missing", "runtime_overlay", "runtime_qmp_socket", "runtime_serial_socket", "runtime_pid_file", "runtime_log_file", "console_host", "console_port"]) delete payload[key];
        const baseName = `${source.name} copy`;
        const names = new Set([...topology.nodes, ...created].map((node) => node.name));
        let name = baseName, suffix = 2;
        while (names.has(name)) name = `${baseName} ${suffix++}`;
        payload.name = name;
        payload.x = Number(source.x ?? 0) + 140;
        payload.y = Number(source.y ?? 0) + 100;
        const node = await api.createNode(props.project.project_id, payload as Omit<Node, "node_id">);
        idMap.set(source.node_id, node.node_id); created.push(node);
      }
      for (const link of copied.links) {
        const endpoints = link.nodes.map((endpoint) => ({ ...endpoint, node_id: idMap.get(endpoint.node_id) ?? "" }));
        if (endpoints.every((endpoint) => endpoint.node_id)) createdLinks.push(await api.createLink(props.project.project_id, endpoints));
      }
      setTopology((current) => ({ ...current, nodes: [...current.nodes, ...created], links: [...current.links, ...createdLinks] }));
      setSelectedNodeIds(created.map((node) => node.node_id));
      props.onSelectCanvas();
      setFeatureMessage(`${created.length} device${created.length === 1 ? "" : "s"} pasted with ${createdLinks.length} internal link${createdLinks.length === 1 ? "" : "s"}.`);
    } catch (reason) {
      setFeatureMessage(`Paste stopped: ${messageFor(reason)}`);
      if (created.length) void api.topology(props.project.project_id).then(setTopology);
    }
  }

  async function toggleSelectedNodeLocks() {
    if (projectLocked) return;
    const selected = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id));
    const lock = selected.some((node) => !node.locked);
    const targets = selected.filter((node) => Boolean(node.locked) !== lock);
    const results = await Promise.allSettled(targets.map((node) => api.updateNode(props.project.project_id, node.node_id, { locked: lock })));
    const updated = results.flatMap((result) => result.status === "fulfilled" ? [result.value] : []);
    setTopology((current) => ({ ...current, nodes: current.nodes.map((node) => updated.find((item) => item.node_id === node.node_id) ?? node) }));
    if (results.some((result) => result.status === "rejected")) setFeatureMessage("Some selected devices could not be locked or unlocked.");
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
      <WorkspaceToolbar workspace={props} topology={topology} selectedNodeIds={selectedNodeIds} selectedLifecycleNodes={selectedLifecycleNodes} projectLocked={projectLocked} gridVisible={gridVisible} setGridVisible={setGridVisible} snapToGrid={snapToGrid} setSnapToGrid={setSnapToGrid} selectedDrawing={selectedDrawing} selectedLink={selectedLink} runBulkLifecycle={runBulkLifecycle} runAllLifecycle={runAllLifecycle} deleteSelectedNodes={deleteSelectedNodes} toggleProjectLock={toggleProjectLock} startDrawingTool={startDrawingTool} setDrawingEditTool={setDrawingEditTool} setDrawingEditValues={setDrawingEditValues} valuesFromDrawing={valuesFromDrawing} setDrawingEditOpen={setDrawingEditOpen} changeDrawingOrder={changeDrawingOrder} removeDrawing={removeDrawing} setSnapshotsOpen={setSnapshotsOpen} setLogOpen={setLogOpen} setScreenshotOpen={setScreenshotOpen} setLinkPropertiesOpen={setLinkPropertiesOpen} setHelpOpen={setHelpOpen} setInfoOpen={setInfoOpen} onToggleConsoleDock={props.onToggleConsoleDock ?? (() => props.onOpenConsoleDock?.())} consoleDockOpen={props.consoleOpen ?? false} onSelectAll={() => { setSelectedNodeIds(topology.nodes.map((node) => node.node_id)); props.onSelectCanvas(); }} onSelectNone={() => { setSelectedNodeIds([]); setSelectedDrawingId(null); props.onSelectCanvas(); }} arrangeSelection={arrangeSelection} toggleSelectedNodeLocks={toggleSelectedNodeLocks} copySelectedNodes={copySelectedNodes} pasteCopiedNodes={() => void pasteCopiedNodes()} topologyDockOpen={topologyDockOpen} serversDockOpen={serversDockOpen} onToggleTopologyDock={() => setTopologyDockOpen(!topologyDockOpen)} onToggleServersDock={() => setServersDockOpen(!serversDockOpen)} zoom={view.zoom} onZoom={(delta) => setView((current) => ({ ...current, zoom: Math.max(0.1, Math.min(maxZoom, Number((current.zoom + delta).toFixed(1)))) }))} onFitTopology={fitTopology} onZoomToSelection={zoomToSelection} onResetZoom={() => setView({ zoom: 1, centerX: 0, centerY: 0 })} showInterfaceLabels={showInterfaceLabels} onToggleInterfaceLabels={() => setShowInterfaceLabels((current) => !current)} />
      <WorkspaceIconToolbar workspace={props} selectedLink={selectedLink} setLinkPropertiesOpen={setLinkPropertiesOpen} startDrawingTool={startDrawingTool} runAllLifecycle={runAllLifecycle} showInterfaceLabels={showInterfaceLabels} onToggleInterfaceLabels={() => setShowInterfaceLabels((current) => !current)} zoomIn={() => setView((current) => ({ ...current, zoom: Math.min(maxZoom, Number((current.zoom + 0.1).toFixed(1))) }))} zoomOut={() => setView((current) => ({ ...current, zoom: Math.max(0.1, Number((current.zoom - 0.1).toFixed(1))) }))} fitTopology={fitTopology} />

      <div className={`sim-workspace-grid ${!topologyDockOpen && !serversDockOpen ? "is-docks-closed" : ""}`}>
        <WorkspaceSidebar workspace={props} deviceQuery={deviceQuery} setDeviceQuery={setDeviceQuery} groupedTemplates={groupedTemplates} />
        <WorkspaceDocks workspace={props} topology={topology} topologyOpen={topologyDockOpen} serversOpen={serversDockOpen} onCloseTopology={() => setTopologyDockOpen(false)} onCloseServers={() => setServersDockOpen(false)} selectedLink={selectedLink} onConfigureNode={() => selectedNode && setConfigureNode(selectedNode)} onOpenLinkProperties={() => setLinkPropertiesOpen(true)} onToggleLinkSuspend={(link) => void toggleLinkSuspend(link)} renderNodeContextMenu={(nodeId) => { const node = topology.nodes.find((item) => item.node_id === nodeId); return node ? <WorkspaceContextMenu workspace={props} selectedNode={node} selectedLink={null} selectedDrawing={null} setConfigureNode={setConfigureNode} setSymbolNode={setSymbolNode} toggleNodeIsolation={toggleNodeIsolation} toggleNodeLock={toggleNodeLock} setLinkPropertiesOpen={setLinkPropertiesOpen} toggleLinkSuspend={toggleLinkSuspend} setDrawingEditTool={setDrawingEditTool} setDrawingEditValues={setDrawingEditValues} valuesFromDrawing={valuesFromDrawing} setDrawingEditOpen={setDrawingEditOpen} changeDrawingOrder={changeDrawingOrder} removeDrawing={removeDrawing} /> : null; }} />
        <Card className="sim-canvas-card">
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div className={`sim-canvas ${props.connecting ? "is-connecting" : ""} ${pendingDrawing ? "is-drawing" : ""}`}>
                {featureMessage && <div className="sim-map-inline-notice" role="status">{featureMessage}</div>}
                  {props.connecting && <div className="sim-map-inline-notice sim-link-mode-banner" role="status">Link mode is on: left-click a device to pick a port. Press Esc or <button type="button" className="underline" onClick={props.onToggleConnect}>exit link mode</button> to get the normal device menu back.</div>}
                <div className="sim-zoom-hint">Ctrl + scroll to zoom · drag with middle mouse to pan · <button type="button" onClick={() => setView({ zoom: 1, centerX: 0, centerY: 0 })}>{Math.round(view.zoom * 100)}%</button></div>

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
                  showInterfaceLabels={showInterfaceLabels}
                  onToggleInterfaceLabels={() => setShowInterfaceLabels((current) => !current)}
                  projectLocked={projectLocked}
                  selectedNodeIds={selectedNodeIds}
                  selectedDrawingId={selectedDrawingId}
                  onCenterChange={(centerX, centerY) => setView((current) => ({ ...current, centerX, centerY }))}
                  selection={props.selection}
                  connecting={props.connecting}
                  linkEndpoints={props.linkEndpoints}
                  onNodeClick={(id) => { setSelectedDrawingId(null); setSelectedNodeIds([id]); props.onSelectNode(id); }}
                  onMultiSelectNode={(id) => setSelectedNodeIds((current) => current.includes(id) ? current.filter((item) => item !== id) : [...current, id])}
                  onMultiSelect={(ids) => { setSelectedDrawingId(null); setSelectedNodeIds(ids); }}
                  onLinkClick={(id) => { setSelectedDrawingId(null); setSelectedNodeIds([]); props.onSelectLink(id); }}
                  onCanvasClick={(point) => { void placeDrawing(point); }}
                  onPortClick={selectLinkEndpoint}
                  onNodeContext={(id) => { setSelectedNodeIds([id]); setSelectedDrawingId(null); props.onNodeContext(id); }}
                  onNodeDoubleClick={(id) => {
                    const node = topology.nodes.find((item) => item.node_id === id);
                    if (!node) return;
                    if (["qemu", "docker", "vpcs", "dynamips"].includes(node.node_type ?? "") && node.console_type !== "none") {
                      props.onOpenConsole(node);
                      return;
                    }
                    setConfigureNode(node);
                  }}
                  onNodeMove={props.onNodeMove}
                  onNodeMoveEnd={props.onNodeMoveEnd}
                  onDrawingClick={(id) => { setSelectedDrawingId(id); setSelectedNodeIds([]); props.onSelectCanvas(); }}
                  onDrawingMove={(id, x, y) => void moveDrawing(id, x, y)}
                  onDeleteSelected={() => { if (selectedDrawing) void removeDrawing(); else if (selectedLink) props.onDeleteSelection(); else deleteSelectedNodes(); }}
                  onCopy={copySelectedNodes}
                  onPaste={() => void pasteCopiedNodes()}
                  onClearSelection={() => { setSelectedNodeIds([]); setSelectedDrawingId(null); props.onSelectCanvas(); }}
                  onZoom={(delta) => setView((current) => ({ ...current, zoom: Math.max(0.1, Math.min(maxZoom, current.zoom + delta)) }))}
                  onNodeLabelMove={(id, x, y) => void saveNodeLabelPosition(id, x, y)}
                  onLinkEndpointLabelMove={(linkId, nodeId, x, y) => void saveLinkEndpointLabelPosition(linkId, nodeId, x, y)}
                  onDropTemplate={(templateId, x, y) => props.onAddTemplate(templateId, x, y)}
                />
              </div>
            </ContextMenuTrigger>
            <WorkspaceContextMenu workspace={props} selectedNode={selectedNode} selectedLink={selectedLink} selectedDrawing={selectedDrawing} setConfigureNode={setConfigureNode} setSymbolNode={setSymbolNode} toggleNodeIsolation={toggleNodeIsolation} toggleNodeLock={toggleNodeLock} setLinkPropertiesOpen={setLinkPropertiesOpen} toggleLinkSuspend={toggleLinkSuspend} setDrawingEditTool={setDrawingEditTool} setDrawingEditValues={setDrawingEditValues} valuesFromDrawing={valuesFromDrawing} setDrawingEditOpen={setDrawingEditOpen} changeDrawingOrder={changeDrawingOrder} removeDrawing={removeDrawing} onOpenCaptureViewer={openCaptureViewer} onOpenWebWireshark={openWebWireshark} onRestartWebWireshark={restartWebWireshark} onStopCapture={stopLinkCapture} onResetLink={resetLink} />
          </ContextMenu>
        </Card>

      </div>
      <ProjectStatusBar topology={topology} projectLocked={projectLocked} connecting={props.connecting} linkEndpointCount={props.linkEndpoints.length} pendingDrawing={pendingDrawing} onCancelDrawing={() => setPendingDrawing(null)} connection={props.connection} onOpenLog={() => setLogOpen(true)} errors={logCounts.errors} warnings={logCounts.warnings} resources={<ResourceMonitorPopover resources={props.resources} runtimeInfo={props.runtimeInfo} error={props.resourceError} refreshing={props.refreshingResources} onRefresh={props.onRefreshResources} />} />
      <NodeSettingsDialog node={configureNode} open={configureNode !== null} onOpenChange={(open) => { if (!open) setConfigureNode(null); }} onSave={props.onConfigureNode} />
      <DrawingEditorDialog tool={drawingEditOpen ? drawingEditTool : drawingTool} open={drawingEditOpen || drawingTool !== null} mode={drawingEditOpen ? "edit" : "create"} initial={drawingEditValues} onOpenChange={(open) => { if (!open) { setDrawingTool(null); setDrawingEditOpen(false); } }} onSave={onDrawingConfigured} />
      <SnapshotsDialog projectId={props.project.project_id} open={snapshotsOpen} onOpenChange={setSnapshotsOpen} onRestored={setTopology} />
      <LinkPropertiesDialog projectId={props.project.project_id} link={selectedLink ?? null} nodes={topology.nodes} open={linkPropertiesOpen} onOpenChange={setLinkPropertiesOpen} onUpdated={(updated) => setTopology((current) => ({ ...current, links: current.links.map((link) => link.link_id === updated.link_id ? updated : link) }))} />
      <SymbolPickerDialog projectId={props.project.project_id} node={symbolNode} open={symbolNode !== null} onOpenChange={(open) => { if (!open) setSymbolNode(null); }} onSaved={(updated) => { setTopology((current) => ({ ...current, nodes: current.nodes.map((node) => node.node_id === updated.node_id ? updated : node) })); if (typeof updated.symbol === "string") setCustomSymbolIds((current) => current.includes(updated.symbol as string) ? current : [...current, updated.symbol as string]); }} />
      <LogConsoleDialog projectId={props.project.project_id} cursor={topology.event_cursor} open={logOpen} onOpenChange={setLogOpen} onCountsChange={onLogCount} />
      <HelpDialog open={helpOpen} onOpenChange={setHelpOpen} />
      <InfoDialog project={props.project} nodeCount={topology.nodes.length} linkCount={topology.links.length} drawingCount={topology.drawings.length} eventCursor={topology.event_cursor} open={infoOpen} onOpenChange={setInfoOpen} />
      <ScreenshotDialog open={screenshotOpen} onOpenChange={setScreenshotOpen} onExport={(request) => void exportScreenshot(request)} pngAvailable={pngScreenshotAvailable()} />
      {captureViewer&&<CaptureWindow url={captureViewer.url} downloadUrl={captureViewer.downloadUrl} title={captureViewer.title} onClose={()=>setCaptureViewer(null)}/>}
      {wiresharkWindows.map((item) => <WebWiresharkWindow key={item.key} projectId={props.project.project_id} linkId={item.linkId} wsPath={item.wsPath} title={item.title} onClose={() => setWiresharkWindows((current) => current.filter((window) => window.linkId !== item.linkId))} />)}
    </section>
  );
}

