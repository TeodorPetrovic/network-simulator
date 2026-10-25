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
  consoleOpen: boolean;
  onCloseTopology: () => void;
  onCloseServers: () => void;
  onCloseConsole: () => void;
  onOpenConsole: () => void;
  selectedLink: Link | null | undefined;
  onConfigureNode: () => void;
  onOpenLinkProperties: () => void;
  onToggleLinkSuspend: (link: Link) => void;
  renderNodeContextMenu: (nodeId: string) => ReactNode;
};

export function WorkspaceDocks({ workspace, topology, topologyOpen, serversOpen, consoleOpen, onCloseTopology, onCloseServers, onCloseConsole, onOpenConsole, selectedLink, onConfigureNode, onOpenLinkProperties, onToggleLinkSuspend, renderNodeContextMenu }: WorkspaceDocksProps) {
  return <aside className="sim-workspace-docks" aria-label="Workspace docks">
    {topologyOpen && <section className="sim-summary-dock" aria-label="Topology Summary">
      <header><b>Topology Summary</b><span>{topology.nodes.length} devices · {topology.links.length} links</span><Button variant="ghost" size="icon-xs" aria-label="Close Topology Summary" onClick={onCloseTopology}><X /></Button></header>
      <ScrollArea className="sim-summary-content">
        {topology.nodes.length ? topology.nodes.map((node) => <div key={node.node_id} className={`sim-summary-node ${workspace.selectedNode?.node_id === node.node_id ? "is-selected" : ""}`}>
          <ContextMenu><ContextMenuTrigger asChild><button className="sim-summary-node-select" onClick={() => workspace.onSelectNode(node.node_id)}>
            <span className={`sim-summary-status ${node.status === "started" ? "is-running" : node.status === "suspended" ? "is-suspended" : ""}`} />
            <span className="sim-summary-node-copy"><b>{node.name}</b><small>{node.node_type ?? "device"}</small></span>
          </button></ContextMenuTrigger>{renderNodeContextMenu(node.node_id)}</ContextMenu>
          {topology.links.filter((link) => link.nodes.some((endpoint) => endpoint.node_id === node.node_id)).map((link) => <div className="sim-summary-link" key={link.link_id}>↳ Link {link.link_id.slice(0, 6)}</div>)}
        </div>) : <p className="sim-dock-empty">No devices on this topology.</p>}
        {workspace.selectedNode && <NodeInspector key={workspace.selectedNode.node_id} node={workspace.selectedNode} links={topology.links} nodes={topology.nodes} onLifecycle={workspace.onNodeLifecycle} onOpenConsole={workspace.onOpenConsole} onRename={workspace.onNodeRename} onConfigure={onConfigureNode} onDelete={workspace.onDeleteSelection} />}
        {selectedLink && <Card className="m-2 p-2"><b className="text-xs">Ethernet segment</b><div className="mt-2 flex flex-wrap gap-1"><Button size="sm" variant="outline" onClick={onOpenLinkProperties}>Link properties</Button><Button size="sm" variant="outline" onClick={onOpenLinkProperties}>Packet capture</Button><Button size="sm" variant="outline" onClick={() => onToggleLinkSuspend(selectedLink)}>{selectedLink.suspend ? "Resume link" : "Suspend link"}</Button></div></Card>}
      </ScrollArea>
    </section>}
    {serversOpen && <ServersSummary resources={workspace.resources} onClose={onCloseServers} />}
    {!consoleOpen ? <button className="sim-console-hidden-tab" onClick={onOpenConsole}>Show Console</button> : <button className="sim-console-hidden-tab" onClick={onCloseConsole}>Hide Console</button>}
  </aside>;
}

function ServersSummary({ resources, onClose }: { resources: ResourceMonitor | null; onClose: () => void }) {
  const host = resources?.host;
  const [expanded, setExpanded] = useState(true);
  return <section className={`sim-summary-dock sim-servers-dock ${expanded ? "" : "is-collapsed"}`} aria-label="Servers Summary">
    <header><b>Servers Summary</b><Button variant="ghost" size="icon-xs" aria-label={expanded ? "Collapse Servers Summary" : "Expand Servers Summary"} onClick={() => setExpanded((value) => !value)}>{expanded ? <ChevronDown /> : <ChevronUp />}</Button><Button variant="ghost" size="icon-xs" aria-label="Close Servers Summary" onClick={onClose}><X /></Button></header>
    {expanded && <div className="sim-servers-content"><div><Cpu /><span>Local server</span><b>{host?.cpu_usage_percent == null ? "—" : `${host.cpu_usage_percent.toFixed(0)}%`}</b><small>CPU</small></div><div><MemoryStick /><span>Memory</span><b>{host?.memory_used_bytes == null || host.memory_total_bytes == null ? "—" : `${(host.memory_used_bytes / 1024 ** 3).toFixed(1)} / ${(host.memory_total_bytes / 1024 ** 3).toFixed(1)} GB`}</b><small>Used / total</small></div></div>}
  </section>;
}
