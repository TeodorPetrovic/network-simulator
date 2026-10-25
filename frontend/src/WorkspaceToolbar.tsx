import { Activity, Camera, Cable, ChevronDown, Circle, HelpCircle, Image, Layers, Link2, Lock, Minus, MoreHorizontal, Network, Pause, Play, RectangleHorizontal, Search, Settings2, Square, Terminal, Trash2, Type, Unlock } from "lucide-react";
import type { Node, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import type { DrawingTool } from "./map/helpers";
import type { DrawingValues } from "./useWorkspaceActions";
import { nodesForLifecycle } from "./lifecycle";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";

type Props = {
  workspace: SimulatorWorkspaceProps; topology: TopologySnapshot; selectedNodeIds: string[]; selectedLifecycleNodes: Node[];
  projectLocked: boolean; gridVisible: boolean; setGridVisible: (value: boolean | ((current: boolean) => boolean)) => void;
  snapToGrid: boolean; setSnapToGrid: (value: boolean | ((current: boolean) => boolean)) => void;
  selectedDrawing: TopologySnapshot["drawings"][number] | null; runBulkLifecycle: (action: "start" | "stop" | "suspend" | "reload") => void;
  deleteSelectedNodes: () => void; toggleProjectLock: () => void; startDrawingTool: (tool: DrawingTool) => void;
  setDrawingEditTool: (tool: DrawingTool | null) => void; setDrawingEditValues: (values: DrawingValues | null) => void;
  valuesFromDrawing: (drawing: TopologySnapshot["drawings"][number]) => DrawingValues; setDrawingEditOpen: (open: boolean) => void;
  changeDrawingOrder: (direction: "front" | "back") => void; removeDrawing: () => void;
  setSnapshotsOpen: (open: boolean) => void; setLogOpen: (open: boolean) => void; setScreenshotOpen: (open: boolean) => void;
  setHelpOpen: (open: boolean) => void; setInfoOpen: (open: boolean) => void;
  zoom: number; onZoom: (delta: number) => void; onFitTopology: () => void; onZoomToSelection: () => void; onResetZoom: () => void;
  showInterfaceLabels: boolean; onToggleInterfaceLabels: () => void;
};

export function WorkspaceToolbar({ workspace: props, topology, selectedNodeIds, selectedLifecycleNodes, projectLocked, gridVisible, setGridVisible, snapToGrid, setSnapToGrid, selectedDrawing, runBulkLifecycle, deleteSelectedNodes, toggleProjectLock, startDrawingTool, setDrawingEditTool, setDrawingEditValues, valuesFromDrawing, setDrawingEditOpen, changeDrawingOrder, removeDrawing, setSnapshotsOpen, setLogOpen, setScreenshotOpen, setHelpOpen, setInfoOpen, zoom, onZoom, onFitTopology, onZoomToSelection, onResetZoom, showInterfaceLabels, onToggleInterfaceLabels }: Props) {
  return (
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
              <DropdownMenuItem onSelect={onToggleInterfaceLabels}>{showInterfaceLabels ? "Hide interface labels" : "Show interface labels"}</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Zoom · {Math.round(zoom * 100)}%</DropdownMenuLabel>
              <DropdownMenuItem onSelect={() => onZoomToSelection()}>Zoom to selection</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onZoom(0.1)}>Zoom in</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onZoom(-0.1)}>Zoom out</DropdownMenuItem>
              <DropdownMenuItem onSelect={onFitTopology}>Fit topology</DropdownMenuItem>
              <DropdownMenuItem onSelect={onResetZoom}>Reset zoom</DropdownMenuItem>
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
              <DropdownMenuItem onSelect={() => setInfoOpen(true)}>Project information…</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button variant="outline" size="icon" aria-label="Map shortcuts" title="Map shortcuts" onClick={() => setHelpOpen(true)}><HelpCircle /></Button>
          <Button variant="outline" onClick={props.onOpenMarketplace}><Search />Image catalog</Button>
          <Button variant="outline" onClick={props.onOpenApplianceManager}><Settings2 />Appliances</Button>
        </div>
      </header>
  );
}
