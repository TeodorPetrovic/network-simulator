import type { Dispatch, SetStateAction } from "react";
import { ChevronDown, Filter, ListTree, Plus, Search, Server, Settings2, Trash2 } from "lucide-react";
import type { Link, Node, RuntimeTemplate, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { portLabel, deviceGroupLabel } from "./map/TopologyCanvas";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { DeviceTemplateCard, NodeInspector } from "./WorkspaceDeviceComponents";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

type Props = {
  workspace: SimulatorWorkspaceProps; topology: TopologySnapshot; rightPanel: string; setRightPanel: (value: string) => void;
  deviceQuery: string; setDeviceQuery: (value: string) => void; groupedTemplates: Array<{ kind: string; templates: RuntimeTemplate[] }>;
  openTemplateGroups: Record<string, boolean>; setOpenTemplateGroups: Dispatch<SetStateAction<Record<string, boolean>>>;
  visibleSummaryNodes: Node[]; summaryQuery: string; setSummaryQuery: (value: string) => void; summaryStatus: string; setSummaryStatus: (value: string) => void;
  setSelectedNodeIds: Dispatch<SetStateAction<string[]>>; selectedNode: Node | null | undefined; selectedLink: Link | null | undefined;
  setConfigureNode: (node: Node | null) => void; setLinkPropertiesOpen: (open: boolean) => void; toggleLinkSuspend: (link: Link) => void;
};

export function WorkspaceSidebar({ workspace: props, topology, rightPanel, setRightPanel, deviceQuery, setDeviceQuery, groupedTemplates, openTemplateGroups, setOpenTemplateGroups, visibleSummaryNodes, summaryQuery, setSummaryQuery, summaryStatus, setSummaryStatus, setSelectedNodeIds, selectedNode, selectedLink, setConfigureNode, setLinkPropertiesOpen, toggleLinkSuspend }: Props) {
  return (
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
  );
}
