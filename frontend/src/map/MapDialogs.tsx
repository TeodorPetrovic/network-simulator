import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { api, projectNotificationsUrl } from "../api";
import type { Link, LinkEndpoint, NetworkSymbol, Node, ProjectSnapshot } from "../api";
import type { DrawingTool } from "./helpers";
import { endpointName, interfacesForNode } from "./helpers";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Unavailable, errorMessage } from "./dialogShared";
import { Textarea } from "@/components/ui/textarea";

export function InterfacePickerDialog({ node, links, open, onOpenChange, onPick }: {
  node: Node | null;
  links: Link[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (endpoint: LinkEndpoint) => void;
}) {
  const [filter, setFilter] = useState("");
  if (!node) return null;
  const interfaces = interfacesForNode(node);
  const normalizedFilter = filter.trim().toLocaleLowerCase();
  const visibleInterfaces = normalizedFilter
    ? interfaces.filter((endpoint) => endpointName(node, endpoint).toLocaleLowerCase().includes(normalizedFilter))
    : interfaces;
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>Choose interface on {node.name}</DialogTitle><DialogDescription>Select a free interface for the new link.</DialogDescription></DialogHeader>
    {interfaces.length > 16 && <Input aria-label="Filter interfaces" placeholder="Search interfaces…" value={filter} onChange={(event) => setFilter(event.target.value)} />}
    <div className="max-h-72 overflow-y-auto"><div className="grid grid-cols-2 gap-2">{visibleInterfaces.map((endpoint) => {
      const connected = links.some((link) => link.nodes.some((item) => item.node_id === endpoint.node_id && item.adapter_number === endpoint.adapter_number && item.port_number === endpoint.port_number));
      return <Button key={`${endpoint.adapter_number}:${endpoint.port_number}`} variant="outline" disabled={connected} onClick={() => { onPick(endpoint); onOpenChange(false); }}>{endpointName(node, endpoint)}{connected ? " · connected" : ""}</Button>;
    })}{visibleInterfaces.length === 0 && <p className="col-span-2 py-3 text-center text-sm text-muted-foreground">No interfaces match this search.</p>}</div></div>
    <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export function SnapshotsDialog({ projectId, open, onOpenChange, onRestored }: {
  projectId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestored: (snapshot: Awaited<ReturnType<typeof api.topology>>) => void;
}) {
  const [snapshots, setSnapshots] = useState<ProjectSnapshot[]>([]);
  const [name, setName] = useState("");
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  async function load() {
    setLoading(true); setError("");
    try { setSnapshots(await api.snapshots(projectId)); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }
  useEffect(() => { if (open) void load(); }, [open, projectId]);

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) return;
    setBusyId("new"); setError(""); setNotice("");
    try { await api.createSnapshot(projectId, name.trim()); setName(""); await load(); setNotice("Snapshot created."); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusyId(""); }
  }
  async function restore(snapshot: ProjectSnapshot) {
    if (!window.confirm(`Restore “${snapshot.name}”? This replaces the current project topology.`)) return;
    setBusyId(snapshot.snapshot_id); setError(""); setNotice("");
    try { await api.restoreSnapshot(projectId, snapshot.snapshot_id); onRestored(await api.topology(projectId)); setNotice(`Restored “${snapshot.name}”.`); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusyId(""); }
  }
  async function remove(snapshot: ProjectSnapshot) {
    setBusyId(snapshot.snapshot_id); setError(""); setNotice("");
    try { await api.deleteSnapshot(projectId, snapshot.snapshot_id); await load(); setNotice("Snapshot deleted."); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusyId(""); }
  }

  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="max-w-xl">
    <DialogHeader><DialogTitle>Project snapshots</DialogTitle><DialogDescription>Save and restore a point-in-time topology snapshot.</DialogDescription></DialogHeader>
    <form className="flex gap-2" onSubmit={(event) => void create(event)}><Input aria-label="Snapshot name" placeholder="Snapshot name" value={name} onChange={(event) => setName(event.target.value)} /><Button type="submit" disabled={!name.trim() || busyId !== ""}>{busyId === "new" ? "Saving…" : "Create snapshot"}</Button></form>
    {error && <Unavailable message={error} />}{notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
    <ScrollArea className="max-h-72">{loading ? <p className="py-6 text-center text-sm text-muted-foreground">Loading snapshots…</p> : snapshots.length ? <div className="space-y-2">{snapshots.map((snapshot) => <div key={snapshot.snapshot_id} className="flex items-center gap-2 rounded-md border p-3"><div className="min-w-0 flex-1"><p className="truncate text-sm font-medium">{snapshot.name}</p><p className="text-xs text-muted-foreground">{snapshot.created_at ?? snapshot.snapshot_id}</p></div><Button size="sm" variant="outline" disabled={Boolean(busyId)} onClick={() => void restore(snapshot)}>Restore</Button><Button size="sm" variant="ghost" disabled={Boolean(busyId)} onClick={() => void remove(snapshot)}>Delete</Button></div>)}</div> : !error && <p className="py-6 text-center text-sm text-muted-foreground">No snapshots saved for this project.</p>}</ScrollArea>
  </DialogContent></Dialog>;
}

export function DrawingEditorDialog({ open, onOpenChange, onSave, tool, mode, initial }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tool: DrawingTool | null;
  mode: "create" | "edit";
  initial: { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string } | null;
  onSave: (tool: DrawingTool, values: { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string }) => void;
}) {
  const [width, setWidth] = useState("180");
  const [height, setHeight] = useState("100");
  const [stroke, setStroke] = useState("#111111");
  const [fill, setFill] = useState("none");
  const [strokeWidth, setStrokeWidth] = useState("1");
  const [text, setText] = useState("Text");
  const [imageDataUrl, setImageDataUrl] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    if (!open) return;
    setError("");
    setWidth(String(initial?.width ?? 180)); setHeight(String(initial?.height ?? 100));
    setStroke(initial?.stroke ?? "#111111"); setFill(initial?.fill ?? "none");
    setStrokeWidth(String(initial?.strokeWidth ?? 1)); setText(initial?.text ?? "Text"); setImageDataUrl(initial?.imageDataUrl ?? "");
  }, [open, tool, initial]);
  if (!tool) return null;
  const title: Record<DrawingTool, string> = { rectangle: "Rectangle", ellipse: "Ellipse", line: "Line", text: "Text", image: "Image" };
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>{mode === "edit" ? "Edit" : "Add"} {title[tool]} drawing</DialogTitle><DialogDescription>{mode === "edit" ? "Update the selected drawing." : "Set the drawing appearance, then click the map to place it."}</DialogDescription></DialogHeader>
    <div className="grid grid-cols-2 gap-3">
      <label className="grid gap-1.5"><Label>Width</Label><Input type="number" min="1" value={width} onChange={(event) => setWidth(event.target.value)} /></label>
      <label className="grid gap-1.5"><Label>Height</Label><Input type="number" min="1" value={height} onChange={(event) => setHeight(event.target.value)} /></label>
      {tool !== "image" && <><label className="grid gap-1.5"><Label>Stroke</Label><Input type="color" value={stroke} onChange={(event) => setStroke(event.target.value)} /></label><label className="grid gap-1.5"><Label>Fill</Label><Input type="color" value={fill === "none" ? "#ffffff" : fill} onChange={(event) => setFill(event.target.value)} /></label><label className="grid gap-1.5"><Label>Stroke width</Label><Input type="number" min="1" max="16" value={strokeWidth} onChange={(event) => setStrokeWidth(event.target.value)} /></label></>}
      {tool === "text" && <label className="col-span-2 grid gap-1.5"><Label>Text</Label><Textarea value={text} onChange={(event) => setText(event.target.value)} rows={3} /></label>}
      {tool === "image" && <label className="col-span-2 grid gap-1.5"><Label>Image file</Label><Input type="file" accept="image/png,image/jpeg,image/gif,image/webp" onChange={(event) => { const file = event.target.files?.[0]; if (!file) return; const reader = new FileReader(); reader.onload = () => setImageDataUrl(typeof reader.result === "string" ? reader.result : ""); reader.onerror = () => setError("Could not read the selected image."); reader.readAsDataURL(file); }} /></label>}
    </div>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button><Button disabled={tool === "image" && !imageDataUrl} onClick={() => { const values = { width: Number(width) || 1, height: Number(height) || 1, stroke, fill, strokeWidth: Number(strokeWidth) || 1, text, ...(imageDataUrl ? { imageDataUrl } : {}) }; onSave(tool, values); onOpenChange(false); }}>{mode === "edit" ? "Save drawing" : "Choose appearance"}</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export function SymbolPickerDialog({ projectId, node, open, onOpenChange, onSaved }: {
  projectId: string;
  node: Node | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (node: Node) => void;
}) {
  const [symbols, setSymbols] = useState<NetworkSymbol[]>([]);
  const [selected, setSelected] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [uploading, setUploading] = useState(false);
  useEffect(() => {
    if (!open) return;
    setLoading(true); setError("");
    void Promise.allSettled([api.listSymbols(), api.defaultSymbols()]).then(([customResult, defaultsResult]) => {
      const custom = customResult.status === "fulfilled" ? customResult.value.map((item) => typeof item === "string" ? { name: item, symbol_id: item } : { ...item, name: item.name ?? item.filename ?? item.symbol_id ?? "Unknown symbol" }) : [];
      const defaults = defaultsResult.status === "fulfilled" ? Object.entries(defaultsResult.value.symbols).map(([name, filename]) => ({ name, filename, symbol_id: filename.startsWith(":/") ? filename : `:/symbols/${filename}` })) : [];
      const seen = new Set(custom.map((item) => item.symbol_id ?? item.filename ?? item.name));
      setSymbols([...custom, ...defaults.filter((item) => !seen.has(item.symbol_id))]);
      if (!custom.length && !defaults.length) setError(`Symbols are not available: ${errorMessage(customResult.status === "rejected" ? customResult.reason : defaultsResult.status === "rejected" ? defaultsResult.reason : null)}`);
    }).finally(() => setLoading(false));
  }, [open]);
  if (!node) return null;
  async function save() {
    if (!node || !selected) return;
    try { const updated = await api.updateNode(projectId, node.node_id, { symbol: selected }); onSaved(updated); onOpenChange(false); }
    catch (reason) { setError(`Could not change symbol: ${errorMessage(reason)}`); }
  }
  async function upload(file?: File) {
    if (!file) return;
    setUploading(true); setError("");
    try {
      await api.uploadProjectSymbol(projectId, file);
      const id = file.name;
      setSelected(id);
      setSymbols((current) => [...current.filter((item) => (item.symbol_id ?? item.filename) !== id), { symbol_id: id, filename: id, name: id, builtin: false }, ...current]);
    } catch (reason) { setError(`Could not upload symbol: ${errorMessage(reason)}`); }
    finally { setUploading(false); }
  }
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>Change symbol for {node.name}</DialogTitle><DialogDescription>Choose a symbol from the controller library.</DialogDescription></DialogHeader>
    {loading && <p className="text-sm text-muted-foreground">Loading symbols…</p>}
    {symbols.length > 0 && <Select value={selected} onValueChange={setSelected}><SelectTrigger aria-label="Node symbol"><SelectValue placeholder="Choose a symbol" /></SelectTrigger><SelectContent>{symbols.map((symbol) => { const value = symbol.symbol_id ?? symbol.path ?? symbol.filename ?? symbol.name ?? "unknown"; return <SelectItem key={value} value={value}>{symbol.name ?? symbol.filename ?? value}</SelectItem>; })}</SelectContent></Select>}
    <label className="grid gap-1.5 text-sm"><Label>Upload custom symbol</Label><Input type="file" accept="image/svg+xml,image/png,image/jpeg" aria-label="Upload custom symbol" disabled={uploading} onChange={(event) => { void upload(event.target.files?.[0]); event.currentTarget.value = ""; }} />{uploading && <span className="text-muted-foreground">Uploading symbol…</span>}</label>
    {error && <Unavailable message={error} />}
    {!loading && !error && symbols.length === 0 && <p className="text-sm text-muted-foreground">The controller returned no symbols.</p>}
    <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>Close</Button><Button disabled={!selected || loading || uploading} onClick={() => void save()}>Apply symbol</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export function LogConsoleDialog({ projectId, cursor, open, onOpenChange, onCountsChange }: { projectId: string; cursor: number; open: boolean; onOpenChange: (open: boolean) => void; onCountsChange?: (level: "error" | "warning") => void }) {
  const [messages, setMessages] = useState<Array<{ sequence: number; action: string; text: string }>>([]);
  const [status, setStatus] = useState("Waiting for controller events");
  useEffect(() => {
    if (typeof WebSocket === "undefined") return;
    let active = true;
    let socket: WebSocket | null = null;
    setStatus("Connecting to project event stream…");
    void projectNotificationsUrl(projectId, cursor).then((url) => {
      if (!active) return;
      socket = new WebSocket(url);
      socket.onopen = () => setStatus("Connected to project event stream");
      socket.onmessage = (event) => {
        try {
          const value = JSON.parse(String(event.data)) as { sequence?: number; action?: string; event?: Record<string, unknown> };
          const action = String(value.action ?? "controller.event");
          const details = String(value.event?.name ?? value.event?.node_id ?? value.event?.link_id ?? value.event?.drawing_id ?? "");
          const severity = String(value.event?.severity ?? value.event?.level ?? value.event?.type ?? "").toLowerCase();
          if (severity.includes("error")) onCountsChange?.("error");
          else if (severity.includes("warn")) onCountsChange?.("warning");
          setMessages((current) => [...current.slice(-199), { sequence: Number(value.sequence ?? 0), action, text: details }]);
        } catch { /* Ignore non-JSON keepalive frames. */ }
      };
      socket.onerror = () => setStatus("The project event stream is not available.");
      socket.onclose = () => setStatus("Project event stream disconnected");
    }).catch(() => setStatus("The project event stream is not available."));
    return () => { active = false; socket?.close(); };
  }, [projectId, cursor, onCountsChange]);
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="max-w-2xl">
    <DialogHeader><DialogTitle>Project log console</DialogTitle><DialogDescription>Recent events from the live project notification stream.</DialogDescription></DialogHeader>
    <p role="status" className="text-xs text-muted-foreground">{status}</p>
    <ScrollArea className="h-80 rounded-md border bg-muted/30 p-3"><div className="space-y-1 font-mono text-xs">{messages.length ? messages.map((message, index) => <div key={`${message.sequence}-${index}`}><span className="mr-2 text-muted-foreground">#{message.sequence}</span><b>{message.action}</b>{message.text && <span> · {message.text}</span>}</div>) : <p className="text-muted-foreground">No new project events yet.</p>}</div></ScrollArea>
  </DialogContent></Dialog>;
}

export function HelpDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>Map shortcuts</DialogTitle><DialogDescription>Keyboard and pointer controls for the project map.</DialogDescription></DialogHeader>
    <div className="grid grid-cols-[1fr_auto] gap-x-5 gap-y-2 text-sm"><span>Select multiple devices</span><kbd>Ctrl / ⌘ + click</kbd><span>Select devices in a region</span><kbd>Drag empty canvas</kbd><span>Copy and paste selected devices</span><kbd>Ctrl / ⌘ + C / V</kbd><span>Nudge selected devices</span><kbd>Arrow keys</kbd><span>Move a device label</span><kbd>Drag its label</kbd><span>Pan the map</span><kbd>Space + drag</kbd><span>Delete selected devices or links</span><kbd>Delete / Backspace</kbd><span>Select all devices</span><kbd>Ctrl / ⌘ + A</kbd><span>Zoom</span><kbd>Ctrl / ⌘ + + / −</kbd><span>Undo last device move</span><kbd>Ctrl / ⌘ + Z</kbd><span>Clear selection</span><kbd>Escape</kbd></div>
    <DialogFooter><Button onClick={() => onOpenChange(false)}>Close</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export function InfoDialog({ project, nodeCount, linkCount, drawingCount, eventCursor, open, onOpenChange }: {
  project: { project_id: string; name: string; status?: string };
  nodeCount: number;
  linkCount: number;
  drawingCount: number;
  eventCursor: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>Project information</DialogTitle><DialogDescription>Current project map details from the controller.</DialogDescription></DialogHeader>
    <dl className="grid grid-cols-[140px_1fr] gap-x-4 gap-y-2 text-sm"><dt className="text-muted-foreground">Project</dt><dd>{project.name}</dd><dt className="text-muted-foreground">Project ID</dt><dd className="break-all font-mono text-xs">{project.project_id}</dd><dt className="text-muted-foreground">Status</dt><dd>{project.status ?? "opened"}</dd><dt className="text-muted-foreground">Devices</dt><dd>{nodeCount}</dd><dt className="text-muted-foreground">Links</dt><dd>{linkCount}</dd><dt className="text-muted-foreground">Drawings</dt><dd>{drawingCount}</dd><dt className="text-muted-foreground">Event cursor</dt><dd>#{eventCursor}</dd></dl>
    <DialogFooter><Button onClick={() => onOpenChange(false)}>Close</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export type ScreenshotRequest = { name: string; format: "svg" | "png" };

export function ScreenshotDialog({ open, onOpenChange, onExport, pngAvailable }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onExport: (request: ScreenshotRequest) => void;
  pngAvailable: boolean;
}) {
  const [name, setName] = useState(`screenshot-${Date.now()}`);
  const [format, setFormat] = useState<ScreenshotRequest["format"]>("svg");
  useEffect(() => { if (open) { setName(`screenshot-${Date.now()}`); setFormat("svg"); } }, [open]);
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle>Take a screenshot</DialogTitle><DialogDescription>Export the current map as a vector SVG or a PNG image.</DialogDescription></DialogHeader>
    <label className="grid gap-1.5"><Label htmlFor="map-screenshot-name">Screenshot name</Label><Input id="map-screenshot-name" value={name} onChange={(event) => setName(event.target.value)} /></label>
    <fieldset className="flex gap-4 text-sm"><legend className="mb-2 font-medium">File type</legend>
      <label className="flex items-center gap-2"><input type="radio" name="map-screenshot-format" value="svg" checked={format === "svg"} onChange={() => setFormat("svg")} />SVG</label>
      <label className="flex items-center gap-2"><input type="radio" name="map-screenshot-format" value="png" checked={format === "png"} disabled={!pngAvailable} onChange={() => setFormat("png")} />PNG{!pngAvailable && <span className="text-muted-foreground">(unavailable)</span>}</label>
    </fieldset>
    <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>Cancel</Button><Button disabled={!name.trim() || (format === "png" && !pngAvailable)} onClick={() => { onExport({ name: name.trim(), format }); onOpenChange(false); }}>Take a screenshot</Button></DialogFooter>
  </DialogContent></Dialog>;
}

export { LinkPropertiesDialog } from "./LinkPropertiesDialog";
