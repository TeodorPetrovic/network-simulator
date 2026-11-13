import { useState } from "react";
import type { ReactNode } from "react";
import { ChevronDown, ChevronUp, Cpu, MemoryStick, X } from "lucide-react";
import type { Link, ResourceMonitor, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Card } from "@/components/ui/card";
import { NodeInspector } from "./WorkspaceDeviceComponents";
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";

export type WorkspaceDocksProps = {
  workspace: SimulatorWorkspaceProps;
  topology: TopologySnapshot;
  topologyOpen: boolean;
  serversOpen: boolean;
  onCloseTopology: () => void;
  onCloseServers: () => void;
  selectedLink: Link | null | undefined;
  onConfigureNode: () => void;
  onOpenLinkProperties: () => void;
  onToggleLinkSuspend: (link: Link) => void;
  renderNodeContextMenu: (nodeId: string) => ReactNode;
};

export function WorkspaceDocks({ workspace, topology, topologyOpen, serversOpen, onCloseTopology, onCloseServers, selectedLink, onConfigureNode, onOpenLinkProperties, onToggleLinkSuspend, renderNodeContextMenu }: WorkspaceDocksProps) {
  return <aside className="fixed right-3 top-16 z-20 flex max-h-[calc(100vh-5rem)] w-80 flex-col gap-2" aria-label="Workspace docks">
    {topologyOpen && <section className="flex min-h-0 flex-col overflow-hidden rounded-lg border bg-card text-card-foreground shadow-lg" aria-label="Topology Summary">
      <header className="flex items-center justify-between gap-2 border-b px-3 py-2"><b className="text-sm">Topology Summary</b><span className="text-xs text-muted-foreground">{topology.nodes.length} devices · {topology.links.length} links</span><Button variant="ghost" size="icon-xs" aria-label="Close Topology Summary" onClick={onCloseTopology}><X /></Button></header>
      <ScrollArea className="min-h-0 flex-1">
        {topology.nodes.length ? topology.nodes.map((node) => <div key={node.node_id} className={workspace.selectedNode?.node_id === node.node_id ? "bg-accent/40" : ""}>
          <ContextMenu><ContextMenuTrigger asChild><button className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-accent/50" onClick={() => workspace.onSelectNode(node.node_id)}>
            <span className={`size-2 shrink-0 rounded-full ${node.status === "started" ? "bg-success" : node.status === "suspended" ? "bg-warning" : "bg-muted-foreground/50"}`} />
            <span className="flex min-w-0 flex-col"><b className="truncate text-xs">{node.name}</b><small className="text-[11px] text-muted-foreground">{node.node_type ?? "device"}</small></span>
          </button></ContextMenuTrigger>{renderNodeContextMenu(node.node_id)}</ContextMenu>
          {workspace.selectedNode?.node_id === node.node_id && <NodeInspector key={node.node_id} node={node} links={topology.links} nodes={topology.nodes} onLifecycle={workspace.onNodeLifecycle} onOpenConsole={workspace.onOpenConsole} onRename={workspace.onNodeRename} onConfigure={onConfigureNode} onDelete={workspace.onDeleteSelection} />}
        </div>) : <p className="p-4 text-center text-xs text-muted-foreground">No devices on this topology.</p>}
        {selectedLink && <Card className="m-2 p-2"><b className="text-xs">Ethernet segment</b><div className="mt-2 flex flex-wrap gap-1"><Button size="sm" variant="outline" onClick={onOpenLinkProperties}>Link properties</Button><Button size="sm" variant="outline" onClick={onOpenLinkProperties}>Packet capture</Button><Button size="sm" variant="outline" onClick={() => onToggleLinkSuspend(selectedLink)}>{selectedLink.suspend ? "Resume link" : "Suspend link"}</Button></div></Card>}
      </ScrollArea>
    </section>}
    {serversOpen && <ServersSummary resources={workspace.resources} onClose={onCloseServers} />}
  </aside>;
}

function ServersSummary({ resources, onClose }: { resources: ResourceMonitor | null; onClose: () => void }) {
  const host = resources?.host;
  const [expanded, setExpanded] = useState(true);
  return <section className="overflow-hidden rounded-lg border bg-card text-card-foreground shadow-lg" aria-label="Servers Summary">
    <header className="flex items-center justify-between border-b px-3 py-2"><b className="text-sm">Servers Summary</b><div><Button variant="ghost" size="icon-xs" aria-label={expanded ? "Collapse Servers Summary" : "Expand Servers Summary"} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown /> : <ChevronUp />}</Button><Button variant="ghost" size="icon-xs" aria-label="Close Servers Summary" onClick={onClose}><X /></Button></div></header>
    {expanded && <div className="grid grid-cols-2 gap-3 p-3 text-xs"><div className="flex flex-col gap-1"><Cpu className="size-4 text-muted-foreground" /><span>Local server</span><b>{host?.cpu_usage_percent == null ? "—" : `${host.cpu_usage_percent.toFixed(0)}%`}</b><small className="text-muted-foreground">CPU</small></div><div className="flex flex-col gap-1"><MemoryStick className="size-4 text-muted-foreground" /><span>Memory</span><b>{host?.memory_used_bytes == null || host.memory_total_bytes == null ? "—" : `${(host.memory_used_bytes / 1024 ** 3).toFixed(1)} / ${(host.memory_total_bytes / 1024 ** 3).toFixed(1)} GB`}</b><small className="text-muted-foreground">Used / total</small></div></div>}
  </section>;
}
