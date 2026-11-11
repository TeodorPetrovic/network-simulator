import { useEffect, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { api, getAuthToken, serverBase } from "../api";
import type { Link, LinkFilterDescription, Node } from "../api";
import { captureDisplayName, defaultCaptureName, defaultEndpointLabelPosition, endpointName } from "./helpers";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import { Unavailable, colorInput, errorMessage } from "./dialogShared";

export function LinkPropertiesDialog({ projectId, link, nodes, open, onOpenChange, onUpdated }: {
  projectId: string;
  link: Link | null;
  nodes: Node[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated: (link: Link) => void;
}) {
  const [color, setColor] = useState("#64748b");
  const [width, setWidth] = useState("2");
  const [filters, setFilters] = useState<Record<string, unknown>>({});
  const [filterDescriptions, setFilterDescriptions] = useState<LinkFilterDescription[]>([]);
  const [captureFileName, setCaptureFileName] = useState("");
  const [captureActive, setCaptureActive] = useState(false);
  const endpointRunning = (link?.nodes ?? []).some((endpoint) => nodes.find((node) => node.node_id === endpoint.node_id)?.status === "started");
  const [webWireshark, setWebWireshark] = useState(false);
  const [webWiresharkEnabled, setWebWiresharkEnabled] = useState(false);
  const [dockerAvailable, setDockerAvailable] = useState(false);
  const [startingWireshark, setStartingWireshark] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [endpointLabels, setEndpointLabels] = useState<Array<{ nodeId: string; text: string; x: string; y: string; rotation: string }>>([]);
  useEffect(() => {
    if (!open || !link) return;
    const style = (link.link_style ?? {}) as Record<string, unknown>;
    setColor(String(style.color ?? "#64748b"));
    setWidth(String(style.width ?? 2));
    setCaptureActive(Boolean(link.capturing));
    setWebWireshark(Boolean(link.wireshark));
    setCaptureFileName(captureDisplayName(link, nodes));
    setFilters(api.linkFilters(link));
    setEndpointLabels(link.nodes.map((endpoint) => {
      const label = endpoint.label;
      const node = nodes.find((item) => item.node_id === endpoint.node_id);
      const defaultPosition = node ? defaultEndpointLabelPosition(node, endpoint) : { x: 8, y: 31 };
      return {
        nodeId: endpoint.node_id,
        text: label?.text ?? (node ? endpointName(node, endpoint) : ""),
        x: String(label?.x ?? defaultPosition.x),
        y: String(label?.y ?? defaultPosition.y),
        rotation: String(label?.rotation ?? 0),
      };
    }));
    setError(""); setNotice("");
    void Promise.allSettled([api.settings(), api.runtimes()]).then(([settings, runtimes]) => {
      setWebWiresharkEnabled(settings.status === "fulfilled" && (settings.value.WebWireshark as { enabled?: boolean } | undefined)?.enabled === true);
      setDockerAvailable(runtimes.status === "fulfilled" && runtimes.value.docker.available);
    });
    void api.availableLinkFilters(projectId, link.link_id).then(setFilterDescriptions).catch((reason) => setError(`Link filters are not available: ${errorMessage(reason)}`));
  }, [open, link?.link_id, projectId, nodes]);
  if (!link) return null;
  const activeLink = link;
  const unavailableFilterError = error.toLowerCase().includes("link filters are not available");
  async function save() {
    if (!link) return;
    setBusy(true); setError(""); setNotice("");
    try {
      const updated = await api.updateLink(projectId, link.link_id, { link_style: { color, width: Number(width) || 1 } });
      onUpdated(updated); setNotice("Link appearance saved.");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  async function saveFilters() {
    setBusy(true); setError(""); setNotice("");
    try { const updated = await api.updateLinkFilters(projectId, activeLink.link_id, filters); setFilters((updated.filters ?? {}) as Record<string, unknown>); onUpdated(updated); setNotice("Link filters saved."); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }
  async function saveEndpointLabels() {
    setBusy(true); setError(""); setNotice("");
    try {
      const updated = await api.updateLink(projectId, activeLink.link_id, {
        nodes: activeLink.nodes.map((endpoint) => {
          const draft = endpointLabels.find((item) => item.nodeId === endpoint.node_id);
          if (!draft) return endpoint;
          return {
            ...endpoint,
            label: {
              ...endpoint.label,
              text: draft.text,
              x: Number(draft.x),
              y: Number(draft.y),
              rotation: Number(draft.rotation) || 0,
            },
          };
        }),
      });
      onUpdated(updated); setNotice("Endpoint labels saved.");
    } catch (reason) { setError(`Endpoint labels could not be saved: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function capture(action: "start" | "stop") {
    setBusy(true); setError(""); setNotice("");
    try {
      if (action === "start") {
        setStartingWireshark(webWireshark);
        const updated = await api.startLinkCapture(projectId, activeLink.link_id, { capture_file_name: captureFileName.trim() || defaultCaptureName(activeLink, nodes), data_link_type: "DLT_EN10MB", ...(webWireshark ? { wireshark: true } : {}) });
        onUpdated(updated);
      } else {
        await api.stopLinkCapture(projectId, activeLink.link_id);
        onUpdated({ ...activeLink, capturing: false });
      }
      setCaptureActive(action === "start"); setNotice(action === "start" ? "Packet capture started." : "Packet capture stopped.");
    }
    catch (reason) { setError(`Packet capture is not available: ${errorMessage(reason)}`); }
    finally { setBusy(false); setStartingWireshark(false); }
  }
  const streamUrl = async () => {
    const path = api.linkCaptureStreamUrl(projectId, activeLink.link_id);
    const url = new URL(path, serverBase());
    if (getAuthToken()) url.searchParams.set("ticket", await api.accessTicket(path));
    return url.toString();
  };
  const openCaptureStream = async () => {
    const popup = window.open("about:blank", "_blank");
    if (popup) popup.opener = null;
    try {
      const url = await streamUrl();
      if (popup) popup.location.assign(url);
      else setError("The browser blocked the capture stream window.");
    } catch (reason) {
      popup?.close();
      setError(`Capture stream is not available: ${errorMessage(reason)}`);
    }
  };
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="max-h-[90vh] w-[min(980px,95vw)] max-w-none overflow-y-auto sm:max-w-none">
    <DialogHeader><DialogTitle>Link properties</DialogTitle><DialogDescription>Change link appearance, filters, and packet capture.</DialogDescription></DialogHeader>
    <Tabs defaultValue="config" className="grid gap-4">
    <TabsList><TabsTrigger value="config">Configuration</TabsTrigger><TabsTrigger value="labels">Endpoint labels</TabsTrigger><TabsTrigger value="capture">Capture</TabsTrigger></TabsList>
    <TabsContent value="config"><div className="grid gap-5 md:grid-cols-2">
      <section className="space-y-3"><h3 className="text-sm font-semibold">Appearance</h3><label className="grid gap-1.5"><Label>Color</Label><Input type="color" value={color} onChange={(event) => setColor(colorInput(event.target.value))} /></label><label className="grid gap-1.5"><Label>Line width</Label><Input type="number" min="1" max="8" value={width} onChange={(event) => setWidth(event.target.value)} /></label><Button onClick={() => void save()} disabled={busy}>Save appearance</Button></section>
      <section className="space-y-3"><h3 className="text-sm font-semibold">Filters</h3>{filterDescriptions.length ? <>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "frequency_drop")}>Drop every Nth packet</span><Input type="number" min="0" value={filterValue(filters.frequency_drop, 0)} onChange={(event) => setFilterValue(setFilters, "frequency_drop", 0, event.target.value)} /></label>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "packet_loss")}>Packet loss (%)</span><Input type="number" min="0" max="100" value={filterValue(filters.packet_loss, 0)} onChange={(event) => setFilterValue(setFilters, "packet_loss", 0, event.target.value)} /></label>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "delay")}>Delay (ms)</span><Input type="number" min="0" value={filterValue(filters.delay, 0)} onChange={(event) => setFilterValue(setFilters, "delay", 0, event.target.value)} /></label>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "delay")}>Jitter (ms)</span><Input type="number" min="0" value={filterValue(filters.delay, 1)} onChange={(event) => setFilterValue(setFilters, "delay", 1, event.target.value)} /></label>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "corrupt")}>Corruption (%)</span><Input type="number" min="0" max="100" value={filterValue(filters.corrupt, 0)} onChange={(event) => setFilterValue(setFilters, "corrupt", 0, event.target.value)} /></label>
        <label className="grid gap-1.5 text-xs"><span title={filterDescription(filterDescriptions, "bpf")}>BPF expression</span><Input value={filterValue(filters.bpf, 0)} onChange={(event) => setFilterValue(setFilters, "bpf", 0, event.target.value, true)} /></label>
        <Button variant="outline" onClick={() => void saveFilters()} disabled={busy}>Save filters</Button>
      </> : <p className="text-sm text-muted-foreground">{unavailableFilterError ? "Filter support is not available on this controller." : "Loading available filters…"}</p>}</section>
    </div></TabsContent>
    <TabsContent value="labels"><section className="grid gap-3"><div><h3 className="text-sm font-semibold">Endpoint labels</h3><p className="text-xs text-muted-foreground">Text and position are stored on each link endpoint. You can also drag a label on the map.</p></div>
      {endpointLabels.map((label, index) => <div key={label.nodeId} className="grid gap-2 rounded-md border p-3 sm:grid-cols-[minmax(100px,1fr)_2fr_72px_72px_72px] sm:items-end">
        <b className="text-xs">{nodes.find((node) => node.node_id === label.nodeId)?.name ?? `Endpoint ${index + 1}`}</b>
        <label className="grid gap-1"><Label htmlFor={`link-label-${label.nodeId}`}>Text</Label><Input id={`link-label-${label.nodeId}`} value={label.text} onChange={(event) => setEndpointLabels((current) => current.map((item) => item.nodeId === label.nodeId ? { ...item, text: event.target.value } : item))} /></label>
        <label className="grid gap-1"><Label htmlFor={`link-label-x-${label.nodeId}`}>X</Label><Input id={`link-label-x-${label.nodeId}`} type="number" value={label.x} onChange={(event) => setEndpointLabels((current) => current.map((item) => item.nodeId === label.nodeId ? { ...item, x: event.target.value } : item))} /></label>
        <label className="grid gap-1"><Label htmlFor={`link-label-y-${label.nodeId}`}>Y</Label><Input id={`link-label-y-${label.nodeId}`} type="number" value={label.y} onChange={(event) => setEndpointLabels((current) => current.map((item) => item.nodeId === label.nodeId ? { ...item, y: event.target.value } : item))} /></label>
        <label className="grid gap-1"><Label htmlFor={`link-label-rotation-${label.nodeId}`}>Rotation</Label><Input id={`link-label-rotation-${label.nodeId}`} type="number" min="-359" max="360" value={label.rotation} onChange={(event) => setEndpointLabels((current) => current.map((item) => item.nodeId === label.nodeId ? { ...item, rotation: event.target.value } : item))} /></label>
      </div>)}
      <Button variant="outline" className="w-fit" onClick={() => void saveEndpointLabels()} disabled={busy || endpointLabels.length !== 2}>Save endpoint labels</Button>
    </section></TabsContent>
    <TabsContent value="capture"><section className="grid gap-2"><label className="grid gap-1.5"><Label>Capture file</Label><Input value={captureFileName} onChange={(event) => setCaptureFileName(event.target.value)} disabled={captureActive} /></label><label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={webWireshark} disabled={!webWiresharkEnabled || !dockerAvailable || captureActive} onChange={(event) => setWebWireshark(event.target.checked)} />Web Wireshark</label>{!webWiresharkEnabled && <p className="text-xs text-muted-foreground">Web Wireshark is disabled in the server settings.</p>}{webWiresharkEnabled && !dockerAvailable && <p className="text-xs text-muted-foreground">Web Wireshark requires Docker to be available.</p>}<div className="flex flex-wrap gap-2"><Badge variant={captureActive ? "default" : "secondary"}>{captureActive ? "Capturing" : "Stopped"}</Badge><Button size="sm" onClick={() => void capture(captureActive ? "stop" : "start")} disabled={busy || (!captureActive && !endpointRunning)}>{captureActive ? "Stop capture" : "Start capture"}</Button><Button size="sm" variant="outline" disabled={!captureActive} onClick={() => void openCaptureStream()}>Open capture stream</Button><Button size="sm" variant="outline" disabled={!captureActive} onClick={() => { void streamUrl().then((url) => copyStreamUrl(url, setNotice, setError)).catch((reason) => setError(`Capture stream is not available: ${errorMessage(reason)}`)); }}>Copy one-use stream URL</Button></div>{!captureActive && !endpointRunning && <p className="text-xs text-muted-foreground">Start a node on this link to enable capture.</p>}{startingWireshark && <p role="status">Starting Web Wireshark… (3-6 s)</p>}</section></TabsContent>
    </Tabs>
    {error && <Unavailable message={error} />}{notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
  </DialogContent></Dialog>;
}


function filterDescription(filters: LinkFilterDescription[], name: string) { return filters.find((item) => item.name === name)?.description ?? ""; }
function filterValue(value: unknown, index: number) { return Array.isArray(value) ? String(value[index] ?? 0) : "0"; }
function setFilterValue(setter: Dispatch<SetStateAction<Record<string, unknown>>>, name: string, index: number, value: string, isText = false) {
  setter((current) => {
    const next = { ...(current[name] && typeof current[name] === "object" ? current[name] as Record<number, unknown> : {}) };
    next[index] = isText ? value : Number(value) || 0;
    return { ...current, [name]: Array.from({ length: Math.max(index + 1, Object.keys(next).length) }, (_, item) => next[item] ?? 0) };
  });
}
async function copyStreamUrl(url: string, onSuccess: (message: string) => void, onError: (message: string) => void) {
  try { await navigator.clipboard.writeText(url); onSuccess("Capture stream URL copied for Wireshark."); }
  catch { onError("Could not copy the stream URL."); }
}
