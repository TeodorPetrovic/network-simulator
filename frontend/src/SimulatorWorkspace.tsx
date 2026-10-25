import { useEffect, useMemo, useState } from "react";
import type { Link, LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { api } from "./api";
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

import { WorkspaceSidebar } from "./WorkspaceSidebar";
import { WorkspaceContextMenu } from "./WorkspaceContextMenu";
import { WorkspaceToolbar } from "./WorkspaceToolbar";
import { useWorkspaceSelection } from "./useWorkspaceSelection";
import { useWorkspaceActions } from "./useWorkspaceActions";

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
  const [showInterfaceLabels, setShowInterfaceLabels] = useState(false);
  const [topology, setTopology] = useState(props.topology);
  const [deviceQuery, setDeviceQuery] = useState("");
  const [summaryQuery, setSummaryQuery] = useState("");
  const [summaryStatus, setSummaryStatus] = useState("all");
  const [rightPanel, setRightPanel] = useState("devices");
  const [openTemplateGroups, setOpenTemplateGroups] = useState<Record<string, boolean>>({});
  const [configureNode, setConfigureNode] = useState<Node | null>(null);
  const [snapshotsOpen, setSnapshotsOpen] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const [screenshotOpen, setScreenshotOpen] = useState(false);
  const [linkPropertiesOpen, setLinkPropertiesOpen] = useState(false);
  const [symbolNode, setSymbolNode] = useState<Node | null>(null);
  const [drawingTool, setDrawingTool] = useState<DrawingTool | null>(null);
  const [pendingDrawing, setPendingDrawing] = useState<{ tool: DrawingTool; values: DrawingValues } | null>(null);
  const [drawingEditOpen, setDrawingEditOpen] = useState(false);
  const [drawingEditTool, setDrawingEditTool] = useState<DrawingTool | null>(null);
  const [drawingEditValues, setDrawingEditValues] = useState<DrawingValues | null>(null);
  const [gridVisible, setGridVisible] = useState(false);
  const [snapToGrid, setSnapToGrid] = useState(true);
  const [projectLocked, setProjectLocked] = useState(Boolean(props.project.locked));
  const [symbolDefaults, setSymbolDefaults] = useState<Record<string, string>>({});
  const [customSymbolIds, setCustomSymbolIds] = useState<string[]>([]);
  const [featureMessage, setFeatureMessage] = useState("");
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
  const { selectedNodeIds, setSelectedNodeIds, selectedDrawingId, setSelectedDrawingId, selectedDrawing } = useWorkspaceSelection(props.selection, topology);
  const selectedNode = topology.nodes.find((node) => node.node_id === (props.selection?.kind === "node" ? props.selection.id : selectedNodeIds[0])) ?? props.selectedNode;
  const selectedLink = props.selection?.kind === "link" ? topology.links.find((link) => link.link_id === props.selection?.id) ?? props.selectedLink : props.selectedLink;
  const { placeDrawing, saveEditedDrawing, removeDrawing, moveDrawing, changeDrawingOrder, toggleProjectLock, toggleNodeLock, toggleNodeIsolation, toggleLinkSuspend, saveNodeLabelPosition, saveLinkEndpointLabelPosition } = useWorkspaceActions({ workspace: props, topology, selectedDrawing, pendingDrawing, projectLocked, setTopology, setSelectedNodeIds, setSelectedDrawingId, setPendingDrawing, setFeatureMessage, setDrawingEditOpen, setProjectLocked });
  const selectedLifecycleNodes = topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && !node.locked);

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
    if (props.selection?.kind === "link") topology.links.find((item) => item.link_id === props.selection?.id)?.nodes.forEach((endpoint) => selectedIds.add(endpoint.node_id));
    const boxes = canvasNodes.filter((node) => selectedIds.has(node.node_id)).map((node) => ({ left: Number(node.x ?? 0) - 56, right: Number(node.x ?? 0) + 56, top: Number(node.y ?? 0) - 36, bottom: Number(node.y ?? 0) + 36 }));
    if (selectedDrawing) { const x = Number(selectedDrawing.x ?? 0); const y = Number(selectedDrawing.y ?? 0); boxes.push({ left: x, right: x + Number(selectedDrawing.width ?? 160), top: y, bottom: y + Number(selectedDrawing.height ?? 90) }); }
    if (!boxes.length) { setFeatureMessage("Select a device, link, or drawing to zoom to it."); return; }
    const left = Math.min(...boxes.map((box) => box.left)); const right = Math.max(...boxes.map((box) => box.right));
    const top = Math.min(...boxes.map((box) => box.top)); const bottom = Math.max(...boxes.map((box) => box.bottom));
    const zoom = Math.max(0.4, Math.min(maxZoom, Math.min(scene.width / (right - left + 80), scene.height / (bottom - top + 80))));
    setView({ zoom, centerX: (left + right) / 2, centerY: (top + bottom) / 2 });
  }

  useEffect(() => {
    fitTopology();
  }, [props.project.project_id]);

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
      <WorkspaceToolbar workspace={props} topology={topology} selectedNodeIds={selectedNodeIds} selectedLifecycleNodes={selectedLifecycleNodes} projectLocked={projectLocked} gridVisible={gridVisible} setGridVisible={setGridVisible} snapToGrid={snapToGrid} setSnapToGrid={setSnapToGrid} selectedDrawing={selectedDrawing} runBulkLifecycle={runBulkLifecycle} deleteSelectedNodes={deleteSelectedNodes} toggleProjectLock={toggleProjectLock} startDrawingTool={startDrawingTool} setDrawingEditTool={setDrawingEditTool} setDrawingEditValues={setDrawingEditValues} valuesFromDrawing={valuesFromDrawing} setDrawingEditOpen={setDrawingEditOpen} changeDrawingOrder={changeDrawingOrder} removeDrawing={removeDrawing} setSnapshotsOpen={setSnapshotsOpen} setLogOpen={setLogOpen} setScreenshotOpen={setScreenshotOpen} setHelpOpen={setHelpOpen} setInfoOpen={setInfoOpen} zoom={view.zoom} onZoom={(delta) => setView((current) => ({ ...current, zoom: Math.max(0.4, Math.min(maxZoom, Number((current.zoom + delta).toFixed(1)))) }))} onFitTopology={fitTopology} onZoomToSelection={zoomToSelection} onResetZoom={() => setView({ zoom: 1, centerX: 0, centerY: 0 })} showInterfaceLabels={showInterfaceLabels} onToggleInterfaceLabels={() => setShowInterfaceLabels((current) => !current)} />

      <div className="sim-workspace-grid">
        <WorkspaceSidebar workspace={props} topology={topology} rightPanel={rightPanel} setRightPanel={setRightPanel} deviceQuery={deviceQuery} setDeviceQuery={setDeviceQuery} groupedTemplates={groupedTemplates} openTemplateGroups={openTemplateGroups} setOpenTemplateGroups={setOpenTemplateGroups} visibleSummaryNodes={visibleSummaryNodes} summaryQuery={summaryQuery} setSummaryQuery={setSummaryQuery} summaryStatus={summaryStatus} setSummaryStatus={setSummaryStatus} setSelectedNodeIds={setSelectedNodeIds} selectedNode={selectedNode} selectedLink={selectedLink} setConfigureNode={setConfigureNode} setLinkPropertiesOpen={setLinkPropertiesOpen} toggleLinkSuspend={toggleLinkSuspend} />
        <Card className="sim-canvas-card">
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div className={`sim-canvas ${props.connecting ? "is-connecting" : ""} ${pendingDrawing ? "is-drawing" : ""}`}>
                {featureMessage && <div className="sim-map-inline-notice" role="status">{featureMessage}</div>}
                <div className="sim-canvas-utility"><ResourceMonitorPopover compact resources={props.resources} runtimeInfo={props.runtimeInfo} error={props.resourceError} refreshing={props.refreshingResources} onRefresh={props.onRefreshResources} /></div>
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
            <WorkspaceContextMenu workspace={props} selectedNode={selectedNode} selectedLink={selectedLink} selectedDrawing={selectedDrawing} setConfigureNode={setConfigureNode} setSymbolNode={setSymbolNode} toggleNodeIsolation={toggleNodeIsolation} toggleNodeLock={toggleNodeLock} setLinkPropertiesOpen={setLinkPropertiesOpen} toggleLinkSuspend={toggleLinkSuspend} setDrawingEditTool={setDrawingEditTool} setDrawingEditValues={setDrawingEditValues} valuesFromDrawing={valuesFromDrawing} setDrawingEditOpen={setDrawingEditOpen} changeDrawingOrder={changeDrawingOrder} removeDrawing={removeDrawing} />
          </ContextMenu>
        </Card>

      </div>
      <NodeSettingsDialog node={configureNode} open={configureNode !== null} onOpenChange={(open) => { if (!open) setConfigureNode(null); }} onSave={props.onConfigureNode} />
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

