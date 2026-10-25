import { Activity, Image, Layers, Lock, Pause, Pencil, Play, Plus, Power, Settings2, Terminal, Trash2, Unlock } from "lucide-react";
import type { Link, Node, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import type { DrawingTool } from "./map/helpers";
import { canApplyLifecycle } from "./lifecycle";
import { availableInterfacesForNode, endpointName, interfacesForNode } from "./map/helpers";
import { nodeHasAuxConsole } from "./map/TopologyCanvas";
import { ContextMenuContent, ContextMenuItem, ContextMenuLabel, ContextMenuSeparator } from "@/components/ui/context-menu";

type Props = {
  workspace: SimulatorWorkspaceProps; selectedNode: Node | null | undefined; selectedLink: Link | null | undefined;
  selectedDrawing: TopologySnapshot["drawings"][number] | null; setConfigureNode: (node: Node) => void; setSymbolNode: (node: Node) => void;
  toggleNodeIsolation: (node: Node) => void; toggleNodeLock: (node: Node) => void; setLinkPropertiesOpen: (open: boolean) => void;
  toggleLinkSuspend: (link: Link) => void; setDrawingEditTool: (tool: DrawingTool | null) => void; setDrawingEditValues: (values: { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string } | null) => void;
  valuesFromDrawing: (drawing: TopologySnapshot["drawings"][number]) => { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string };
  setDrawingEditOpen: (open: boolean) => void; changeDrawingOrder: (direction: "front" | "back") => void; removeDrawing: () => void;
};

export function WorkspaceContextMenu({ workspace: props, selectedNode, selectedLink, selectedDrawing, setConfigureNode, setSymbolNode, toggleNodeIsolation, toggleNodeLock, setLinkPropertiesOpen, toggleLinkSuspend, setDrawingEditTool, setDrawingEditValues, valuesFromDrawing, setDrawingEditOpen, changeDrawingOrder, removeDrawing }: Props) {
  if (props.connecting && selectedNode) {
    const ports = interfacesForNode(selectedNode);
    const available = new Set(availableInterfacesForNode(selectedNode, props.topology.links).map((port) => `${port.adapter_number}:${port.port_number}`));
    return <ContextMenuContent className="w-56"><ContextMenuLabel>{selectedNode.name} · Select port</ContextMenuLabel><ContextMenuSeparator />{ports.map((port) => {
      const used = !available.has(`${port.adapter_number}:${port.port_number}`);
      return <ContextMenuItem key={`${port.adapter_number}:${port.port_number}`} disabled={used} onSelect={() => props.onSelectPort(port)}>{endpointName(selectedNode, port)}{used ? " · in use" : ""}</ContextMenuItem>;
    })}</ContextMenuContent>;
  }
  return (
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
  );
}
