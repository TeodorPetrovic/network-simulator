import { Activity, Camera, Cable, Circle, Image, Minus, Network, Pause, Play, RectangleHorizontal, Scan, Square, Terminal, Type, ZoomIn, ZoomOut } from "lucide-react";
import type { Link, Node, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import type { DrawingTool } from "./map/helpers";
import type { DrawingValues } from "./useWorkspaceActions";
import { nodesForLifecycle } from "./lifecycle";
import { Menubar, MenubarContent, MenubarItem, MenubarMenu, MenubarSeparator, MenubarTrigger } from "@/components/ui/menubar";
import { Button } from "@/components/ui/button";
import type { ReactNode } from "react";

type Props = {
  workspace: SimulatorWorkspaceProps; topology: TopologySnapshot; selectedNodeIds: string[]; selectedLifecycleNodes: Node[];
  projectLocked: boolean; gridVisible: boolean; setGridVisible: (value: boolean | ((current: boolean) => boolean)) => void;
  snapToGrid: boolean; setSnapToGrid: (value: boolean | ((current: boolean) => boolean)) => void;
  selectedDrawing: TopologySnapshot["drawings"][number] | null; runBulkLifecycle: (action: "start" | "stop" | "suspend" | "reload") => void;
  deleteSelectedNodes: () => void; toggleProjectLock: () => void; startDrawingTool: (tool: DrawingTool) => void;
  runAllLifecycle: (action: "start" | "stop" | "suspend" | "reload") => void;
  setDrawingEditTool: (tool: DrawingTool | null) => void; setDrawingEditValues: (values: DrawingValues | null) => void;
  valuesFromDrawing: (drawing: TopologySnapshot["drawings"][number]) => DrawingValues; setDrawingEditOpen: (open: boolean) => void;
  changeDrawingOrder: (direction: "front" | "back") => void; removeDrawing: () => void;
  setSnapshotsOpen: (open: boolean) => void; setLogOpen: (open: boolean) => void; setScreenshotOpen: (open: boolean) => void; setLinkPropertiesOpen: (open: boolean) => void; selectedLink: Link | null | undefined;
  setHelpOpen: (open: boolean) => void; setInfoOpen: (open: boolean) => void;
  onToggleConsoleDock: () => void; consoleDockOpen: boolean;
  onSelectAll: () => void; onSelectNone: () => void;
  arrangeSelection: (mode: "left" | "right" | "top" | "bottom" | "horizontal" | "vertical" | "distribute-horizontal" | "distribute-vertical") => void;
  toggleSelectedNodeLocks: () => void; copySelectedNodes: () => void; pasteCopiedNodes: () => void;
  topologyDockOpen: boolean; serversDockOpen: boolean; onToggleTopologyDock: () => void; onToggleServersDock: () => void;
  zoom: number; onZoom: (delta: number) => void; onResetZoom: () => void; onFitTopology: () => void; onZoomToSelection: () => void; showInterfaceLabels: boolean; onToggleInterfaceLabels: () => void;
};

export function WorkspaceToolbar({ workspace: props, topology, selectedNodeIds, selectedLifecycleNodes, projectLocked, gridVisible, setGridVisible, snapToGrid, setSnapToGrid, selectedDrawing, selectedLink, runBulkLifecycle, runAllLifecycle, deleteSelectedNodes, toggleProjectLock, startDrawingTool, setDrawingEditTool, setDrawingEditValues, valuesFromDrawing, setDrawingEditOpen, changeDrawingOrder, removeDrawing, setSnapshotsOpen, setLogOpen, setScreenshotOpen, setLinkPropertiesOpen, setHelpOpen, setInfoOpen, onToggleConsoleDock, consoleDockOpen, onSelectAll, onSelectNone, arrangeSelection, toggleSelectedNodeLocks, copySelectedNodes, pasteCopiedNodes, topologyDockOpen, serversDockOpen, onToggleTopologyDock, onToggleServersDock, zoom, onZoom, onResetZoom, onFitTopology, onZoomToSelection, showInterfaceLabels, onToggleInterfaceLabels }: Props) {
  const supported = (label: string, action: () => void, disabled = false) => <MenubarItem disabled={disabled} onSelect={action}>{label}</MenubarItem>;
  const draw = (tool: DrawingTool, label: string) => supported(label, () => startDrawingTool(tool));
  const selected = selectedLifecycleNodes.length > 0;
  return <header className="sim-toolbar" aria-label="Project workspace menu">
    <div className="sim-toolbar-brand"><span className="sim-project-mark"><Network size={16} /></span><strong>{props.project.name}</strong></div>
    <Menubar>
      <MenubarMenu><MenubarTrigger>File</MenubarTrigger><MenubarContent>
        {supported("New blank project", () => {}, true)}{supported("Open project…", () => {}, true)}
        <MenubarSeparator />{supported("Save project as…", () => {}, true)}{supported("Export portable project…", () => {}, true)}{supported("Import portable project…", () => {}, true)}
        <MenubarSeparator />{supported("Snapshots…", () => setSnapshotsOpen(true))}{supported("Close project / Back to projects", props.onBack)}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Edit</MenubarTrigger><MenubarContent>
        {supported("Select all", onSelectAll, !topology.nodes.length)}{supported("Select none", onSelectNone, !selectedNodeIds.length)}{supported("Copy selected", copySelectedNodes, !selectedNodeIds.length)}{supported("Paste devices", pasteCopiedNodes, projectLocked)}{supported("Duplicate selected", () => { copySelectedNodes(); pasteCopiedNodes(); }, !selectedNodeIds.length || projectLocked)}<MenubarSeparator />{supported("Preferences…", props.onOpenPreferences)}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Arrange</MenubarTrigger><MenubarContent>
        {supported("Align left", () => arrangeSelection("left"), selectedNodeIds.length < 2 || projectLocked)}{supported("Align right", () => arrangeSelection("right"), selectedNodeIds.length < 2 || projectLocked)}{supported("Align top", () => arrangeSelection("top"), selectedNodeIds.length < 2 || projectLocked)}{supported("Align bottom", () => arrangeSelection("bottom"), selectedNodeIds.length < 2 || projectLocked)}{supported("Align horizontal centers", () => arrangeSelection("horizontal"), selectedNodeIds.length < 2 || projectLocked)}{supported("Align vertical centers", () => arrangeSelection("vertical"), selectedNodeIds.length < 2 || projectLocked)}<MenubarSeparator />{supported("Distribute horizontally", () => arrangeSelection("distribute-horizontal"), selectedNodeIds.length < 3 || projectLocked)}{supported("Distribute vertically", () => arrangeSelection("distribute-vertical"), selectedNodeIds.length < 3 || projectLocked)}<MenubarSeparator />{supported(selectedNodeIds.length > 0 && topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id)).every((node) => node.locked) ? "Unlock selected" : "Lock selected", toggleSelectedNodeLocks, !selectedNodeIds.length || projectLocked)}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>View</MenubarTrigger><MenubarContent>
        {supported(`Zoom in · ${Math.round(zoom * 100)}%`, () => onZoom(0.1))}{supported("Zoom out", () => onZoom(-0.1))}{supported("Reset zoom", onResetZoom)}{supported("Fit to window", onFitTopology)}{supported("Zoom to selection", onZoomToSelection)}
        <MenubarSeparator />{supported(showInterfaceLabels ? "Hide interface labels" : "Show interface labels", onToggleInterfaceLabels)}{supported(gridVisible ? "Hide grid" : "Show grid", () => setGridVisible((value) => !value))}
        {supported(snapToGrid ? "Snap to grid ✓" : "Snap to grid", () => setSnapToGrid((value) => !value))}{supported(projectLocked ? "Unlock project" : "Lock project", () => void toggleProjectLock())}
        <MenubarSeparator />{supported(`${consoleDockOpen ? "Hide" : "Show"} Console dock`, onToggleConsoleDock)}{supported(`Topology summary${topologyDockOpen ? " ✓" : ""}`, onToggleTopologyDock)}{supported(`Servers summary${serversDockOpen ? " ✓" : ""}`, onToggleServersDock)}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Control</MenubarTrigger><MenubarContent>
        {supported("Start all", props.onStartAll, props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "start").length)}
        {supported("Suspend all", () => runAllLifecycle("suspend"), props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "suspend").length)}
        {supported("Stop all", props.onStopAll, props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "stop").length)}
        {supported("Reload all", () => runAllLifecycle("reload"), props.batchLifecycleRunning || !nodesForLifecycle(topology.nodes, "reload").length)}
        {supported("Console to all devices", () => topology.nodes.filter((node) => node.status === "started").forEach((node) => props.onOpenConsole(node)), !topology.nodes.some((node) => node.status === "started"))}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Node</MenubarTrigger><MenubarContent>
        {supported("Start selected", () => runBulkLifecycle("start"), !selected || !nodesForLifecycle(selectedLifecycleNodes, "start").length)}
        {supported("Suspend selected", () => runBulkLifecycle("suspend"), !selected || !nodesForLifecycle(selectedLifecycleNodes, "suspend").length)}
        {supported("Stop selected", () => runBulkLifecycle("stop"), !selected || !nodesForLifecycle(selectedLifecycleNodes, "stop").length)}
        {supported("Reload selected", () => runBulkLifecycle("reload"), !selected || !nodesForLifecycle(selectedLifecycleNodes, "reload").length)}
        {supported("Console selected", () => topology.nodes.filter((node) => selectedNodeIds.includes(node.node_id) && node.status === "started").forEach((node) => props.onOpenConsole(node)), !topology.nodes.some((node) => selectedNodeIds.includes(node.node_id) && node.status === "started"))}
        <MenubarSeparator />{supported("Delete selected", deleteSelectedNodes, !selectedNodeIds.length)}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Annotate</MenubarTrigger><MenubarContent>
        {draw("text", "Add a note / text")}{draw("image", "Add image")}{draw("rectangle", "Rectangle")}{draw("ellipse", "Ellipse")}{draw("line", "Line")}
        {selectedDrawing && <><MenubarSeparator />{supported("Edit selected drawing…", () => { setDrawingEditTool((selectedDrawing.drawing_kind as DrawingTool) ?? "rectangle"); setDrawingEditValues(valuesFromDrawing(selectedDrawing)); setDrawingEditOpen(true); })}{supported("Bring drawing forward", () => void changeDrawingOrder("front"))}{supported("Send drawing backward", () => void changeDrawingOrder("back"))}{supported("Delete drawing", () => void removeDrawing())}</>}
      </MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Tools</MenubarTrigger><MenubarContent>{supported("Packet capture…", () => setLinkPropertiesOpen(true), !selectedLink)}{supported("Image catalog…", props.onOpenMarketplace)}{supported("Appliances…", props.onOpenApplianceManager)}{supported("Project screenshot…", () => setScreenshotOpen(true))}{supported("Project log…", () => setLogOpen(true))}</MenubarContent></MenubarMenu>
      <MenubarMenu><MenubarTrigger>Help</MenubarTrigger><MenubarContent>{supported("Keyboard shortcuts", () => setHelpOpen(true))}{supported("About…", () => setInfoOpen(true))}</MenubarContent></MenubarMenu>
    </Menubar>
  </header>;
}

export function WorkspaceIconToolbar({ workspace, startDrawingTool, runAllLifecycle, selectedLink, setLinkPropertiesOpen, zoomIn, zoomOut, fitTopology, showInterfaceLabels, onToggleInterfaceLabels }: { workspace: SimulatorWorkspaceProps; startDrawingTool: (tool: DrawingTool) => void; runAllLifecycle: (action: "start" | "stop" | "suspend" | "reload") => void; selectedLink: Link | null | undefined; setLinkPropertiesOpen: (open: boolean) => void; zoomIn: () => void; zoomOut: () => void; fitTopology: () => void; showInterfaceLabels: boolean; onToggleInterfaceLabels: () => void }) {
  return <div className="sim-icon-toolbar" role="toolbar" aria-label="Workspace tools">
    <IconAction label="Start all" icon={<Play />} onClick={workspace.onStartAll} disabled={workspace.batchLifecycleRunning} />
    <IconAction label="Suspend all" icon={<Pause />} onClick={() => runAllLifecycle("suspend")} disabled={workspace.batchLifecycleRunning || !nodesForLifecycle(workspace.topology.nodes, "suspend").length} />
    <IconAction label="Stop all" icon={<Square />} onClick={workspace.onStopAll} />
    <IconAction label="Reload all" icon={<Activity />} onClick={() => runAllLifecycle("reload")} disabled={workspace.batchLifecycleRunning || !nodesForLifecycle(workspace.topology.nodes, "reload").length} />
    <IconAction label="Console all" icon={<Terminal />} onClick={() => workspace.topology.nodes.filter((node) => node.status === "started").forEach((node) => workspace.onOpenConsole(node))} disabled={!workspace.topology.nodes.some((node) => node.status === "started")} />
    <span className="sim-toolbar-divider" />
    <IconAction label={showInterfaceLabels ? "Hide interface labels" : "Show interface labels"} icon={<Network />} onClick={onToggleInterfaceLabels} pressed={showInterfaceLabels} />
    <span className="sim-toolbar-divider" />
    <IconAction label="Zoom in" icon={<ZoomIn />} onClick={zoomIn} /><IconAction label="Zoom out" icon={<ZoomOut />} onClick={zoomOut} /><IconAction label="Fit to window" icon={<Scan />} onClick={fitTopology} />
    <span className="sim-toolbar-divider" />
    <IconAction label="Add note" icon={<Type />} onClick={() => startDrawingTool("text")} /><IconAction label="Add image" icon={<Image />} onClick={() => startDrawingTool("image")} /><IconAction label="Rectangle" icon={<RectangleHorizontal />} onClick={() => startDrawingTool("rectangle")} /><IconAction label="Ellipse" icon={<Circle />} onClick={() => startDrawingTool("ellipse")} /><IconAction label="Line" icon={<Minus />} onClick={() => startDrawingTool("line")} />
    <span className="sim-toolbar-divider" />
    <IconAction label="Packet capture" icon={<Camera />} disabled={!selectedLink} onClick={() => setLinkPropertiesOpen(true)} /><IconAction label={workspace.connecting ? "Cancel link" : "Draw link"} icon={<Cable />} onClick={workspace.onToggleConnect} pressed={workspace.connecting} />
  </div>;
}

function IconAction({ label, icon, onClick, disabled = false, pressed = false }: { label: string; icon: ReactNode; onClick?: () => void; disabled?: boolean; pressed?: boolean }) {
  return <Button variant={pressed ? "secondary" : "ghost"} size="icon-sm" className="sim-icon-action" title={label} aria-label={label} aria-pressed={pressed} disabled={disabled} onClick={onClick}>{icon}</Button>;
}
