import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, Plus, Save, Trash2 } from "lucide-react";
import type { Node, Project, TopologySnapshot } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type BuiltinType = "ethernet_switch" | "ethernet_hub" | "cloud" | "nat";
type SwitchPort = { port_number: number; vlan: number; type: string; ethertype: string };
type UdpTunnel = { name: string; lport: string; rhost: string; rport: string };
const labels: Record<BuiltinType, string> = { ethernet_switch: "Ethernet switch", ethernet_hub: "Ethernet hub", cloud: "Cloud", nat: "NAT" };
const typeAliases: Record<BuiltinType, string[]> = { ethernet_switch: ["ethernet_switch"], ethernet_hub: ["ethernet_hub", "hub"], cloud: ["cloud"], nat: ["nat"] };
const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";
const str = (value: unknown, fallback = "") => value === undefined || value === null ? fallback : String(value);

export function BuiltinNodeEditorPage({ nodeType: requestedType }: { nodeType: string }) {
  const navigate = useNavigate();
  const nodeType: BuiltinType = ["ethernet_switch", "ethernet_hub", "cloud", "nat"].includes(requestedType) ? requestedType as BuiltinType : "cloud";
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState("");
  const [topology, setTopology] = useState<TopologySnapshot | null>(null);
  const [nodeId, setNodeId] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [name, setName] = useState("");
  const [consoleType, setConsoleType] = useState("none");
  const [portCount, setPortCount] = useState("8");
  const [switchPorts, setSwitchPorts] = useState<SwitchPort[]>([]);
  const [newPort, setNewPort] = useState<SwitchPort>({ port_number: 0, vlan: 1, type: "access", ethertype: "0x8100" });
  const [ethernetInterfaces, setEthernetInterfaces] = useState<string[]>([]);
  const [ethernetInterface, setEthernetInterface] = useState("");
  const [tapInterfaces, setTapInterfaces] = useState<string[]>([]);
  const [tapInterface, setTapInterface] = useState("");
  const [udpTunnels, setUdpTunnels] = useState<UdpTunnel[]>([]);
  const [udpTunnel, setUdpTunnel] = useState<UdpTunnel>({ name: "", lport: "20000", rhost: "127.0.0.1", rport: "20000" });
  const [remoteConsoleHost, setRemoteConsoleHost] = useState("");
  const [remoteConsolePort, setRemoteConsolePort] = useState("");
  const [remoteConsolePath, setRemoteConsolePath] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => { api.listProjects().then((items) => { setProjects(items); if (items[0]) setProjectId(items[0].project_id); }).catch((reason) => setError(`Project list unavailable: ${errorMessage(reason)}`)); }, []);
  useEffect(() => {
    if (!projectId) { setTopology(null); return; }
    let current = true; setLoading(true); setError(""); setNodeId("");
    api.topology(projectId).then((snapshot) => { if (current) setTopology(snapshot); }).catch((reason) => { if (current) setError(`Project topology unavailable: ${errorMessage(reason)}`); }).finally(() => current && setLoading(false));
    return () => { current = false; };
  }, [projectId]);
  const choices = useMemo(() => topology?.nodes.filter((node) => typeAliases[nodeType].includes(node.node_type ?? "")) ?? [], [topology, nodeType]);
  const selectedNode = choices.find((node) => node.node_id === nodeId);

  useEffect(() => {
    if (!selectedNode) { setName(""); setSwitchPorts([]); return; }
    const properties = (selectedNode.properties && typeof selectedNode.properties === "object" ? selectedNode.properties : {}) as Record<string, unknown>;
    const mappings = Array.isArray(selectedNode.ports_mapping)
      ? selectedNode.ports_mapping as Array<Record<string, unknown>>
      : Array.isArray(properties.ports_mapping) ? properties.ports_mapping as Array<Record<string, unknown>> : [];
    setName(selectedNode.name);
    setConsoleType(str(nodeType === "cloud" ? selectedNode.remote_console_type : selectedNode.console_type, "none"));
    setPortCount(str(mappings.length || properties.ports || properties.number_of_ports || 8));
    setSwitchPorts(mappings.map((row, index) => ({ port_number: Number(row.port_number ?? index), vlan: Number(row.vlan ?? 1), type: str(row.type, "access"), ethertype: str(row.ethertype, "0x8100") })));
    setEthernetInterfaces(mappings.filter((row) => row.type === "ethernet").map((row) => str(row.interface)).filter(Boolean));
    setTapInterfaces(mappings.filter((row) => row.type === "tap").map((row) => str(row.interface)).filter(Boolean));
    setUdpTunnels(mappings.filter((row) => row.type === "udp").map((row) => ({ name: str(row.name), lport: str(row.lport), rhost: str(row.rhost), rport: str(row.rport) })));
    setRemoteConsoleHost(str(selectedNode.remote_console_host ?? properties.remote_console_host, "127.0.0.1"));
    setRemoteConsolePort(str(selectedNode.remote_console_port ?? properties.remote_console_port, "23"));
    setRemoteConsolePath(str(selectedNode.remote_console_http_path ?? properties.remote_console_http_path, "/"));
  }, [selectedNode]);

  function addSwitchPort() { setSwitchPorts((current) => [...current.filter((port) => port.port_number !== Number(newPort.port_number)), { ...newPort, port_number: Number(newPort.port_number), vlan: Number(newPort.vlan) }]); }
  function addUdpTunnel() { if (!udpTunnel.name.trim() || !udpTunnel.rhost.trim()) return; setUdpTunnels((current) => [...current, udpTunnel]); setUdpTunnel((current) => ({ ...current, name: "" })); }
  async function save() {
    if (!selectedNode || !projectId) return;
    setSaving(true); setError(""); setNotice("");
    const patch: Record<string, unknown> = { name: name.trim() };
    if (nodeType === "ethernet_switch") {
      patch.console_type = consoleType;
      patch.ports_mapping = switchPorts.slice().sort((left, right) => left.port_number - right.port_number);
    } else if (nodeType === "ethernet_hub") {
      const count = Number(portCount);
      if (!Number.isInteger(count) || count < 1 || count > 256) {
        setError("Hub port count must be from 1 to 256."); setSaving(false); return;
      }
      patch.ports_mapping = Array.from({ length: count }, (_, port_number) => ({ name: `Ethernet${port_number}`, port_number }));
    } else if (nodeType === "cloud") {
      patch.ports_mapping = [
        ...udpTunnels.map((tunnel) => ({ name: tunnel.name, type: "udp", lport: Number(tunnel.lport), rhost: tunnel.rhost, rport: Number(tunnel.rport) })),
        ...ethernetInterfaces.map((name) => ({ name, type: "ethernet", interface: name })),
        ...tapInterfaces.map((name) => ({ name, type: "tap", interface: name })),
      ];
      patch.remote_console_host = remoteConsoleHost;
      patch.remote_console_port = Number(remoteConsolePort);
      patch.remote_console_type = consoleType;
      patch.remote_console_http_path = remoteConsolePath;
    }
    try {
      await api.configureNode(projectId, selectedNode.node_id, patch);
      setTopology(await api.topology(projectId));
      setNotice(`${labels[nodeType]} settings saved.`);
    } catch (reason) {
      setError(`Settings could not be saved: ${errorMessage(reason)}`);
    } finally { setSaving(false); }
  }

  return <PagesShell active="Preferences"><div className="pages-content"><PageTitle eyebrow="BUILT-IN NODE EDITOR" title={`${labels[nodeType]} configurator`} description="GNS3-style configuration fields for an existing project node." actions={<Button variant="outline" onClick={() => void navigate({ to: "/preferences" as never })}><ArrowLeft size={15} />Preferences</Button>} />
    <div className="pages-notice"><Badge variant="outline">Live node</Badge><span>Settings save through the controller as flat node fields. Connected switch VLAN settings update while the node stays running.</span></div>
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}</div>}
    <Card className="node-editor-card"><div className="node-editor-selectors"><label className="grid gap-1.5"><Label htmlFor="editor-project">Project</Label><Select value={projectId} onValueChange={setProjectId}><SelectTrigger id="editor-project"><SelectValue placeholder="Choose project" /></SelectTrigger><SelectContent>{projects.map((project) => <SelectItem key={project.project_id} value={project.project_id}>{project.name}</SelectItem>)}</SelectContent></Select></label><label className="grid gap-1.5"><Label htmlFor="editor-node">Node</Label><Select value={nodeId} onValueChange={setNodeId} disabled={!topology || loading}><SelectTrigger id="editor-node"><SelectValue placeholder={loading ? "Loading nodes…" : `Choose ${labels[nodeType].toLocaleLowerCase()}`} /></SelectTrigger><SelectContent>{choices.map((node: Node) => <SelectItem key={node.node_id} value={node.node_id}>{node.name}</SelectItem>)}</SelectContent></Select></label></div>
      {projectId && !loading && !choices.length && <div className="pages-empty-state">No {labels[nodeType].toLocaleLowerCase()} nodes exist in this project. Create one from an available template first.</div>}
      {selectedNode && <div className="node-editor-form"><fieldset disabled={saving} className="contents"><label className="grid max-w-lg gap-1.5"><Label htmlFor="builtin-node-name">Name</Label><Input id="builtin-node-name" value={name} onChange={(event) => setName(event.target.value)} /></label>
        {nodeType === "ethernet_switch" && <><label className="grid max-w-lg gap-1.5"><Label htmlFor="switch-console-type">Console type</Label><Select value={consoleType} onValueChange={setConsoleType}><SelectTrigger id="switch-console-type"><SelectValue /></SelectTrigger><SelectContent>{["none", "telnet"].map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent></Select></label><div><div className="mb-2 flex items-center justify-between"><div><h3 className="font-medium">Ethernet ports</h3><p className="text-xs text-muted-foreground">Port number, VLAN, access/dot1q/qinq mode, and EtherType.</p></div></div><Table><TableHeader><TableRow><TableHead>Port number</TableHead><TableHead>VLAN</TableHead><TableHead>Mode</TableHead><TableHead>EtherType</TableHead><TableHead>Actions</TableHead></TableRow></TableHeader><TableBody>{switchPorts.map((port) => <TableRow key={port.port_number}><TableCell>{port.port_number}</TableCell><TableCell>{port.vlan}</TableCell><TableCell>{port.type}</TableCell><TableCell>{port.ethertype}</TableCell><TableCell><Button size="icon-sm" variant="ghost" aria-label={`Delete switch port ${port.port_number}`} onClick={() => setSwitchPorts((current) => current.filter((item) => item.port_number !== port.port_number))}><Trash2 size={14} /></Button></TableCell></TableRow>)}</TableBody></Table><div className="node-editor-inline-fields"><label className="grid gap-1"><Label htmlFor="switch-port-number">Port</Label><Input id="switch-port-number" type="number" min="0" value={newPort.port_number} onChange={(event) => setNewPort((current) => ({ ...current, port_number: Number(event.target.value) }))} /></label><label className="grid gap-1"><Label htmlFor="switch-port-vlan">VLAN</Label><Input id="switch-port-vlan" type="number" min="1" max="4094" value={newPort.vlan} onChange={(event) => setNewPort((current) => ({ ...current, vlan: Number(event.target.value) }))} /></label><label className="grid gap-1"><Label htmlFor="switch-port-mode">Mode</Label><Select value={newPort.type} onValueChange={(value) => setNewPort((current) => ({ ...current, type: value }))}><SelectTrigger id="switch-port-mode"><SelectValue /></SelectTrigger><SelectContent>{["access", "dot1q", "qinq"].map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent></Select></label><label className="grid gap-1"><Label htmlFor="switch-port-ethertype">EtherType</Label><Select value={newPort.ethertype} onValueChange={(value) => setNewPort((current) => ({ ...current, ethertype: value }))}><SelectTrigger id="switch-port-ethertype"><SelectValue /></SelectTrigger><SelectContent>{["0x8100", "0x88a8"].map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent></Select></label><Button className="self-end" type="button" variant="outline" onClick={addSwitchPort}><Plus size={14} />Add port</Button></div></div></>}
        {nodeType === "ethernet_hub" && <label className="grid max-w-lg gap-1.5"><Label htmlFor="hub-port-count">Number of ports</Label><Input id="hub-port-count" type="number" min="1" max="256" value={portCount} onChange={(event) => setPortCount(event.target.value)} /></label>}
        {nodeType === "cloud" && <Tabs defaultValue="ethernet"><TabsList><TabsTrigger value="ethernet">Ethernet interfaces</TabsTrigger><TabsTrigger value="tap">TAP interfaces</TabsTrigger><TabsTrigger value="udp">UDP tunnels</TabsTrigger><TabsTrigger value="misc">Miscellaneous</TabsTrigger></TabsList><TabsContent value="ethernet" className="grid gap-3 pt-3"><p className="text-sm text-muted-foreground">Host Ethernet interface names are entered explicitly.</p><div className="flex gap-2"><Input aria-label="Ethernet interface" placeholder="Ethernet interface" value={ethernetInterface} onChange={(event) => setEthernetInterface(event.target.value)} /><Button type="button" variant="outline" onClick={() => { if (ethernetInterface.trim()) setEthernetInterfaces((current) => [...current, ethernetInterface.trim()]); setEthernetInterface(""); }}><Plus size={14} />Add</Button></div><div className="flex flex-wrap gap-2">{ethernetInterfaces.map((name, index) => <Badge key={`${name}-${index}`} variant="secondary">{name}<button className="ml-1" aria-label={`Remove ${name}`} onClick={() => setEthernetInterfaces((current) => current.filter((_, row) => row !== index))}>×</button></Badge>)}</div></TabsContent><TabsContent value="tap" className="grid gap-3 pt-3"><div className="flex gap-2"><Input aria-label="TAP interface" placeholder="TAP interface" value={tapInterface} onChange={(event) => setTapInterface(event.target.value)} /><Button type="button" variant="outline" onClick={() => { if (tapInterface.trim()) setTapInterfaces((current) => [...current, tapInterface.trim()]); setTapInterface(""); }}><Plus size={14} />Add</Button></div><div className="flex flex-wrap gap-2">{tapInterfaces.map((name, index) => <Badge key={`${name}-${index}`} variant="secondary">{name}<button className="ml-1" aria-label={`Remove ${name}`} onClick={() => setTapInterfaces((current) => current.filter((_, row) => row !== index))}>×</button></Badge>)}</div></TabsContent><TabsContent value="udp" className="grid gap-3 pt-3"><Table><TableHeader><TableRow><TableHead>Name</TableHead><TableHead>Local port</TableHead><TableHead>Remote host</TableHead><TableHead>Remote port</TableHead><TableHead>Actions</TableHead></TableRow></TableHeader><TableBody>{udpTunnels.map((tunnel, index) => <TableRow key={`${tunnel.name}-${index}`}><TableCell>{tunnel.name}</TableCell><TableCell>{tunnel.lport}</TableCell><TableCell>{tunnel.rhost}</TableCell><TableCell>{tunnel.rport}</TableCell><TableCell><Button size="icon-sm" variant="ghost" aria-label={`Delete UDP tunnel ${tunnel.name}`} onClick={() => setUdpTunnels((current) => current.filter((_, row) => row !== index))}><Trash2 size={14} /></Button></TableCell></TableRow>)}</TableBody></Table><div className="node-editor-inline-fields"><label className="grid gap-1"><Label htmlFor="udp-name">Name</Label><Input id="udp-name" value={udpTunnel.name} onChange={(event) => setUdpTunnel((current) => ({ ...current, name: event.target.value }))} /></label><label className="grid gap-1"><Label htmlFor="udp-lport">Local port</Label><Input id="udp-lport" type="number" value={udpTunnel.lport} onChange={(event) => setUdpTunnel((current) => ({ ...current, lport: event.target.value }))} /></label><label className="grid gap-1"><Label htmlFor="udp-rhost">Remote host</Label><Input id="udp-rhost" value={udpTunnel.rhost} onChange={(event) => setUdpTunnel((current) => ({ ...current, rhost: event.target.value }))} /></label><label className="grid gap-1"><Label htmlFor="udp-rport">Remote port</Label><Input id="udp-rport" type="number" value={udpTunnel.rport} onChange={(event) => setUdpTunnel((current) => ({ ...current, rport: event.target.value }))} /></label><Button className="self-end" type="button" variant="outline" onClick={addUdpTunnel}><Plus size={14} />Add tunnel</Button></div></TabsContent><TabsContent value="misc" className="grid gap-3 pt-3 sm:grid-cols-2"><label className="grid gap-1.5"><Label htmlFor="cloud-console-type">Console type</Label><Select value={consoleType} onValueChange={setConsoleType}><SelectTrigger id="cloud-console-type"><SelectValue /></SelectTrigger><SelectContent>{["none", "telnet", "ssh", "vnc", "spice", "http", "https"].map((value) => <SelectItem key={value} value={value}>{value}</SelectItem>)}</SelectContent></Select></label><label className="grid gap-1.5"><Label htmlFor="cloud-console-host">Console host</Label><Input id="cloud-console-host" value={remoteConsoleHost} onChange={(event) => setRemoteConsoleHost(event.target.value)} /></label><label className="grid gap-1.5"><Label htmlFor="cloud-console-port">Console port</Label><Input id="cloud-console-port" type="number" value={remoteConsolePort} onChange={(event) => setRemoteConsolePort(event.target.value)} /></label><label className="grid gap-1.5"><Label htmlFor="cloud-console-path">Console HTTP path</Label><Input id="cloud-console-path" value={remoteConsolePath} onChange={(event) => setRemoteConsolePath(event.target.value)} /></label></TabsContent></Tabs>}
        {nodeType === "nat" && <p className="text-sm text-muted-foreground">NAT node configuration has no fields beyond its name in the upstream configurator.</p>}
        <div className="flex justify-end border-t pt-4"><Button onClick={() => void save()} disabled={saving}><Save size={15} />{saving ? "Saving…" : "Apply"}</Button></div>
        </fieldset>
      </div>}
    </Card>
  </div></PagesShell>;
}
