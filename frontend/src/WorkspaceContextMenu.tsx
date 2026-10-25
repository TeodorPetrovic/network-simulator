import { Activity, Image, Layers, Lock, Pause, Pencil, Play, Plus, Power, Settings2, Terminal, Trash2, Unlock } from "lucide-react";
import type { Link, Node, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import type { DrawingTool } from "./map/helpers";
import { canApplyLifecycle } from "./lifecycle";
import type { LifecycleAction } from "./lifecycle";
import { availableInterfacesForNode, endpointName, interfacesForNode } from "./map/helpers";
import { nodeHasAuxConsole } from "./map/TopologyCanvas";
import { ContextMenuContent, ContextMenuItem, ContextMenuLabel, ContextMenuSeparator } from "@/components/ui/context-menu";

type Props = {
  workspace: SimulatorWorkspaceProps; selectedNode: Node | null | undefined; selectedLink: Link | null | undefined;
  selectedDrawing: TopologySnapshot["drawings"][number] | null; setConfigureNode: (node: Node) => void; setSymbolNode: (node: Node) => void;
  toggleNodeIsolation: (node: Node) => void; toggleNodeLock: (node: Node) => void; setLinkPropertiesOpen: (open: boolean) => void;
  toggleLinkSuspend: (link: Link) => void; setDrawingEditTool: (tool: DrawingTool | null) => void; setDrawingEditValues: (values: { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string } | null) => void;
  onOpenCaptureViewer?: (link: Link, newTab?: boolean) => void;
  onOpenWebWireshark?: (link: Link, newTab?: boolean) => void;
  onRestartWebWireshark?: (link: Link) => void;
  onStopCapture?: (link: Link) => void;
  valuesFromDrawing: (drawing: TopologySnapshot["drawings"][number]) => { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string };
  setDrawingEditOpen: (open: boolean) => void; changeDrawingOrder: (direction: "front" | "back") => void; removeDrawing: () => void;
};

function canShowNodeLifecycle(node: Node, action: LifecycleAction) {
  if (["ethernet_switch", "ethernet_hub", "hub"].includes(node.node_type ?? "")) {
    if (action === "start") return node.status !== "started";
    if (action === "stop") return node.status === "started" || node.status === "suspended";
    return node.status === "started";
  }
  return canApplyLifecycle(node, action);
}

export function WorkspaceContextMenu({ workspace: props, selectedNode, selectedLink, selectedDrawing, setConfigureNode, setSymbolNode, toggleNodeIsolation, toggleNodeLock, setLinkPropertiesOpen, toggleLinkSuspend, setDrawingEditTool, setDrawingEditValues, valuesFromDrawing, setDrawingEditOpen, changeDrawingOrder, removeDrawing, onOpenCaptureViewer, onOpenWebWireshark, onRestartWebWireshark, onStopCapture }: Props) {
  if (props.connecting && selectedNode) {
    const ports = interfacesForNode(selectedNode);
    const available = new Set(availableInterfacesForNode(selectedNode, props.topology.links).map((port) => `${port.adapter_number}:${port.port_number}`));
    return <ContextMenuContent className="w-56"><ContextMenuLabel>{selectedNode.name} · Select port</ContextMenuLabel><ContextMenuSeparator />{ports.map((port) => {
      const used = !available.has(`${port.adapter_number}:${port.port_number}`);
      return <ContextMenuItem key={`${port.adapter_number}:${port.port_number}`} disabled={used} onSelect={() => props.onSelectPort(port)}>{endpointName(selectedNode, port)}{used ? " · in use" : ""}</ContextMenuItem>;
    })}<ContextMenuSeparator /><ContextMenuItem onSelect={() => props.onToggleConnect()}>Exit link mode (show device menu)</ContextMenuItem></ContextMenuContent>;
  }
  return (
            <ContextMenuContent className="w-56">
              <ContextMenuLabel>{selectedNode?.name ?? (selectedLink ? "Ethernet link" : selectedDrawing ? "Drawing" : "Topology")}</ContextMenuLabel>
              <ContextMenuSeparator />
              {selectedNode ? <>
                {canShowNodeLifecycle(selectedNode, "start") && <ContextMenuItem disabled={Boolean(selectedNode.runtime_missing)} onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "start")}><Play />{selectedNode.status === "suspended" ? "Resume device" : "Start device"}</ContextMenuItem>}
                {canShowNodeLifecycle(selectedNode, "stop") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "stop")}><Power />Stop device</ContextMenuItem>}
                {canShowNodeLifecycle(selectedNode, "suspend") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "suspend")}><Pause />Suspend device</ContextMenuItem>}
                {canShowNodeLifecycle(selectedNode, "reload") && <ContextMenuItem onSelect={() => props.onNodeLifecycle(selectedNode.node_id, "reload")}><Activity />Reload device</ContextMenuItem>}
                {selectedNode.status === "started" && ["qemu", "docker", "vpcs", "dynamips"].includes(selectedNode.node_type ?? "") && selectedNode.console_type !== "none" && <ContextMenuItem onSelect={() => props.onOpenConsole(selectedNode)}><Terminal />Open console</ContextMenuItem>}
                {selectedNode.status === "started" && ["qemu", "docker", "vpcs", "dynamips"].includes(selectedNode.node_type ?? "") && nodeHasAuxConsole(selectedNode) && <ContextMenuItem onSelect={() => props.onOpenConsole(selectedNode, true)}><Terminal />Open auxiliary console</ContextMenuItem>}
                <ContextMenuItem onSelect={() => { document.querySelector<HTMLButtonElement>("[data-tab=topology]")?.click(); window.setTimeout(() => document.querySelector<HTMLInputElement>("[data-rename-node]")?.focus(), 0); }}><Pencil />Edit device name</ContextMenuItem>
                {["qemu", "docker", "vpcs", "dynamips", "ethernet_switch", "ethernet_hub", "hub", "cloud", "nat"].includes(selectedNode.node_type ?? "") && <ContextMenuItem onSelect={() => setConfigureNode(selectedNode)}><Settings2 />Configure device</ContextMenuItem>}
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
                <ContextMenuItem onSelect={() => onOpenCaptureViewer?.(selectedLink)}><Activity />Open built-in Capture Viewer</ContextMenuItem>
                <ContextMenuItem onSelect={() => onOpenCaptureViewer?.(selectedLink, true)}>Open in new tab</ContextMenuItem>
                {selectedLink.wireshark === true && <><ContextMenuItem onSelect={() => onOpenWebWireshark?.(selectedLink)}>Open Web Wireshark (Inline)</ContextMenuItem><ContextMenuItem onSelect={() => onOpenWebWireshark?.(selectedLink, true)}>Open Web Wireshark in new tab</ContextMenuItem><ContextMenuItem onSelect={() => onRestartWebWireshark?.(selectedLink)}>Restart Web Wireshark</ContextMenuItem><ContextMenuItem variant="destructive" onSelect={() => onStopCapture?.(selectedLink)}>Stop capture</ContextMenuItem></>}
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
