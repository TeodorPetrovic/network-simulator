import { useEffect, useMemo, useRef, useState } from "react";
import type { RefObject } from "react";
import { Download, FileUp, Pause, Play, RefreshCw, Trash2, ChevronDown, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { Field, Layer } from "./dissect";
import type { PacketRecord } from "./filter";

const PACKET_ROW_HEIGHT = 27;
const OVERSCAN = 12;

export function CaptureToolbar({
  title,
  paused,
  autoScroll,
  relativeTime,
  hasDownload,
  fileInput,
  onToggleTime,
  onToggleScroll,
  onTogglePause,
  onFollow,
  onClear,
  onSave,
  onOpenSaved,
}: {
  title: string;
  paused: boolean;
  autoScroll: boolean;
  relativeTime: boolean;
  hasDownload: boolean;
  fileInput: RefObject<HTMLInputElement | null>;
  onToggleTime: () => void;
  onToggleScroll: () => void;
  onTogglePause: () => void;
  onFollow: () => void;
  onClear: () => void;
  onSave: () => void;
  onOpenSaved: () => void;
}) {
  return <div className="capture-toolbar">
    <strong>{title}</strong><span className="capture-spacer" />
    <Button size="sm" variant="outline" onClick={onToggleTime}>{relativeTime ? "Relative time" : "Absolute time"}</Button>
    <Button size="sm" variant="outline" onClick={onToggleScroll}>{autoScroll ? "Auto-scroll on" : "Auto-scroll off"}</Button>
    <Button size="sm" variant="outline" onClick={onTogglePause}>{paused ? <Play /> : <Pause />}{paused ? "Resume" : "Pause"}</Button>
    <Button size="sm" variant="outline" onClick={onFollow}>Follow stream</Button>
    <Button size="sm" variant="outline" onClick={onClear}><Trash2 />Clear</Button>
    <Button size="sm" variant="outline" onClick={onSave}><Download />Save .pcap</Button>
    {hasDownload && <Button size="sm" variant="outline" onClick={onOpenSaved}><RefreshCw />Open saved capture</Button>}
    <Button size="sm" variant="outline" onClick={() => fileInput.current?.click()}><FileUp />Open file</Button>
  </div>;
}

export function CaptureStatusBar({ status, displayed, total }: { status: string; displayed: number; total: number }) {
  return <footer className="capture-status-bar"><span>{status}</span><span>{displayed} displayed / {total} packets</span></footer>;
}

export function PacketList({
  packets,
  selectedNumber,
  relativeTime,
  autoScroll,
  onSelect,
  onApplyFilter,
}: {
  packets: PacketRecord[];
  selectedNumber: number | null;
  relativeTime: boolean;
  autoScroll: boolean;
  onSelect: (packet: PacketRecord) => void;
  onApplyFilter: (filter: string) => void;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const scrollContainer = useRef<HTMLDivElement>(null);
  const start = Math.max(0, Math.floor(scrollTop / PACKET_ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(packets.length, start + 60 + OVERSCAN * 2);
  const firstTimestamp = packets[0]?.timestamp ?? 0;
  const rows = packets.slice(start, end);

  useEffect(() => {
    if (autoScroll && scrollContainer.current) scrollContainer.current.scrollTop = scrollContainer.current.scrollHeight;
  }, [autoScroll, packets.length]);

  useEffect(() => {
    const selectedIndex = packets.findIndex((packet) => packet.number === selectedNumber);
    const container = scrollContainer.current;
    if (selectedIndex < 0 || !container) return;
    const top = selectedIndex * PACKET_ROW_HEIGHT;
    const bottom = top + PACKET_ROW_HEIGHT;
    if (top < container.scrollTop) container.scrollTop = top;
    else if (bottom > container.scrollTop + container.clientHeight) container.scrollTop = bottom - container.clientHeight;
  }, [packets, selectedNumber]);

  return <div className="capture-list-frame">
    <div className="capture-head" role="row">
      <span>No.</span><span>Time</span><span>Source</span><span>Destination</span><span>Protocol</span><span>Length</span><span>Info</span>
    </div>
    <div ref={scrollContainer} className="capture-rows" role="grid" aria-label="Packets" onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}>
      <div style={{ height: start * PACKET_ROW_HEIGHT }} />
      {rows.map((packet) => <button
        key={packet.number}
        role="row"
        aria-selected={selectedNumber === packet.number}
        className={`capture-row ${selectedNumber === packet.number ? "selected" : ""} protocol-${packet.decoded.protocol.toLowerCase()}`}
        onClick={() => onSelect(packet)}
        onContextMenu={(event) => {
          const target = (event.target as HTMLElement).closest("[data-column]") as HTMLElement | null;
          const column = target?.dataset.column;
          if (!column) return;
          const filters: Record<string, string> = {
            source: packet.decoded.src.includes(":") ? `ipv6.addr == ${packet.decoded.src}` : `ip.addr == ${packet.decoded.src}`,
            destination: packet.decoded.dst.includes(":") ? `ipv6.addr == ${packet.decoded.dst}` : `ip.addr == ${packet.decoded.dst}`,
            protocol: packet.decoded.protocol.toLowerCase(),
          };
          if (filters[column]) { event.preventDefault(); onApplyFilter(filters[column]); }
        }}
      >
        <span data-column="number">{packet.number}</span>
        <span data-column="time">{relativeTime ? (packet.timestamp - firstTimestamp).toFixed(6) : packet.timestamp.toFixed(6)}</span>
        <span data-column="source" title={packet.decoded.src}>{packet.decoded.src || "—"}</span>
        <span data-column="destination" title={packet.decoded.dst}>{packet.decoded.dst || "—"}</span>
        <span data-column="protocol">{packet.decoded.protocol}</span>
        <span data-column="length">{packet.originalLength}</span>
        <span data-column="info" title={packet.decoded.info}>{packet.decoded.info}</span>
      </button>)}
      <div style={{ height: Math.max(0, (packets.length - end) * PACKET_ROW_HEIGHT) }} />
    </div>
  </div>;
}

export function PacketDetails({
  packet,
  onApplyFilter,
}: {
  packet?: PacketRecord;
  onApplyFilter: (filter: string) => void;
}) {
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set([0, 1, 2, 3, 4, 5]));
  const [highlight, setHighlight] = useState<Field | null>(null);
  const layers = packet?.decoded.layers ?? [];
  const fields = layers.flatMap((layer) => layer.fields);

  return <div className="capture-inspector">
    <div className="capture-tree" role="tree" aria-label="Packet details">
      {layers.map((layer, index) => <div key={`${layer.name}-${index}`}>
        <button className="capture-layer" aria-expanded={expanded.has(index)} onClick={() => {
          setExpanded((current) => {
            const next = new Set(current);
            if (next.has(index)) next.delete(index);
            else next.add(index);
            return next;
          });
        }}>
          {expanded.has(index) ? <ChevronDown /> : <ChevronRight />}{layer.name} · {layer.summary}
        </button>
        {expanded.has(index) && layer.fields.map((field) => <button
          key={`${field.label}-${field.byteOffset}`}
          className="capture-field"
          onMouseEnter={() => setHighlight(field)}
          onFocus={() => setHighlight(field)}
          onClick={() => setHighlight(field)}
          onContextMenu={(event) => {
            event.preventDefault();
            onApplyFilter(filterForField(layer, field));
          }}
        >{field.label}: {field.value}</button>)}
      </div>)}
    </div>
    <HexPane packet={packet} fields={fields} highlight={highlight} onHighlight={setHighlight} />
  </div>;
}

function filterForField(layer: Layer, field: Field): string {
  const ip = layer.name.includes("IPv6") ? "ipv6" : "ip";
  if (field.label === "Source Address") return `${ip}.src == ${field.value}`;
  if (field.label === "Destination Address") return `${ip}.dst == ${field.value}`;
  const transport = layer.name.includes("User Datagram") ? "udp" : "tcp";
  if (field.label === "Source Port") return `${transport}.srcport == ${field.value}`;
  if (field.label === "Destination Port") return `${transport}.dstport == ${field.value}`;
  if (field.label.includes("MAC") || field.label === "Source" || field.label === "Destination") {
    return `eth.addr == ${field.value}`;
  }
  return `${layer.name.toLowerCase().replaceAll(" ", ".")} contains "${field.value}"`;
}

export function HexPane({
  packet,
  fields,
  highlight,
  onHighlight,
}: {
  packet?: PacketRecord;
  fields: Field[];
  highlight: Field | null;
  onHighlight: (field: Field | null) => void;
}) {
  const bytes = packet?.data ?? new Uint8Array();
  const rows = Math.ceil(bytes.length / 16);
  return <div className="capture-hex" aria-label="Packet bytes">
    {Array.from({ length: rows }, (_, row) => {
      const start = row * 16;
      const chunk = bytes.subarray(start, start + 16);
      const ascii = Array.from(chunk, (value) => value >= 32 && value < 127 ? String.fromCharCode(value) : ".").join("");
      return <div className="hex-row" key={row}>
        <span>{start.toString(16).padStart(4, "0")}</span>
        <code>{Array.from(chunk, (value, index) => {
          const selected = highlight && start + index >= highlight.byteOffset && start + index < highlight.byteOffset + highlight.length;
          const byteField = fields.find((field) => start + index >= field.byteOffset && start + index < field.byteOffset + field.length) ?? null;
          return <button key={index} className={selected ? "highlight" : ""} aria-label={`Byte ${start + index}: ${value.toString(16).padStart(2, "0")}`} onMouseEnter={() => onHighlight(byteField)} onFocus={() => onHighlight(byteField)}>{value.toString(16).padStart(2, "0")} </button>;
        })}</code>
        <code>{ascii}</code>
      </div>;
    })}
  </div>;
}

export function CaptureFilterBar({
  value,
  error,
  onChange,
  inputRef,
}: {
  value: string;
  error: string;
  onChange: (value: string) => void;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  return <div className={`capture-filter ${error ? "invalid" : ""}`}>
    <label htmlFor="capture-display-filter">Display filter</label>
    <input ref={inputRef} id="capture-display-filter" aria-label="Display filter" value={value} onChange={(event) => onChange(event.target.value)} placeholder="tcp.port == 80 && ip" />
    <span role={error ? "alert" : undefined}>{error || "Filter"}</span>
  </div>;
}

export function StatisticsPanel({ packets }: { packets: PacketRecord[] }) {
  const statistics = useMemo(() => {
    const protocols = new Map<string, { packets: number; bytes: number }>();
    const conversations = new Map<string, { packets: number; bytes: number; start: number; end: number; protocol: string }>();
    const graph = new Map<number, number>();
    for (const packet of packets) {
      const transport = packet.decoded.layers.find((layer) => layer.name.includes("Transmission Control") || layer.name.includes("User Datagram"));
      const names = packet.decoded.layers.map((layer) => layer.name.includes("Ethernet II") ? "Ethernet" :
        layer.name.includes("Internet Protocol Version 4") ? "IPv4" :
          layer.name.includes("Internet Protocol Version 6") ? "IPv6" :
            layer.name.includes("Transmission Control") ? "TCP" :
              layer.name.includes("User Datagram") ? "UDP" : layer.name);
      if (!names.includes(packet.decoded.protocol)) names.push(packet.decoded.protocol);
      for (const name of new Set(names)) {
        const protocol = protocols.get(name) ?? { packets: 0, bytes: 0 };
        protocol.packets += 1;
        protocol.bytes += packet.originalLength;
        protocols.set(name, protocol);
      }
      const ports = transport?.fields.filter((field) => field.label.endsWith("Port")).map((field) => field.value) ?? [];
      const endpointA = `${packet.decoded.src}:${ports[0] ?? ""}`;
      const endpointB = `${packet.decoded.dst}:${ports[1] ?? ""}`;
      const endpoints = [endpointA, endpointB].sort();
      const protocol = transport?.name.includes("Transmission Control") ? "TCP" : transport ? "UDP" : packet.decoded.protocol;
      const key = `${protocol} ${endpoints.join(" ↔ ")}`;
      const conversation = conversations.get(key) ?? { packets: 0, bytes: 0, start: packet.timestamp, end: packet.timestamp, protocol };
      conversation.packets += 1;
      conversation.bytes += packet.originalLength;
      conversation.start = Math.min(conversation.start, packet.timestamp);
      conversation.end = Math.max(conversation.end, packet.timestamp);
      conversations.set(key, conversation);
      const second = Math.floor(packet.timestamp - (packets[0]?.timestamp ?? packet.timestamp));
      graph.set(second, (graph.get(second) ?? 0) + 1);
    }
    return { protocols, conversations, graph };
  }, [packets]);

  return <div className="capture-stats">
    <h3>Protocol Hierarchy</h3>
    {[...statistics.protocols].map(([name, value]) => <p key={name}>{name}<span>{value.packets} packets · {value.bytes} bytes</span></p>)}
    <h3>Conversations</h3>
    {[...statistics.conversations].map(([name, value]) => <p key={name}>{name}<span>{value.packets} packets · {value.bytes} bytes · {(value.end - value.start).toFixed(3)} s</span></p>)}
    <h3>Packets per second</h3>
    <div className="capture-io-graph" aria-label="Packets per second">
      {[...statistics.graph].sort(([a], [b]) => a - b).map(([second, count]) => <span key={second} title={`${second}s: ${count} packets`} style={{ height: `${Math.max(3, count * 12)}px` }} />)}
    </div>
  </div>;
}

export function FollowStreamDialog({
  packets,
  onClose,
}: {
  packets: PacketRecord[];
  onClose: () => void;
}) {
  const [format, setFormat] = useState<"ascii" | "hex" | "raw">("ascii");
  const [direction, setDirection] = useState<"both" | "client" | "server">("both");
  const client = streamDirection(packets, true);
  const server = streamDirection(packets, false);
  const clientText = renderStream(client, format);
  const serverText = renderStream(server, format);
  const byteCount = client.length + server.length;
  return <div className="capture-dialog-backdrop" role="presentation" onClick={onClose}>
    <section role="dialog" aria-modal="true" aria-label="Follow stream" className="capture-follow-dialog" onClick={(event) => event.stopPropagation()}>
      <header><strong>Follow stream</strong><span />
        <label>Format <select value={format} onChange={(event) => setFormat(event.target.value as typeof format)}><option value="ascii">ASCII</option><option value="hex">Hex dump</option><option value="raw">Raw</option></select></label>
        <label>Direction <select value={direction} onChange={(event) => setDirection(event.target.value as typeof direction)}><option value="both">Both</option><option value="client">Client → server</option><option value="server">Server → client</option></select></label>
        <button onClick={() => onClose()}>Close</button>
      </header>
      <div className="capture-follow-streams">
        {direction !== "server" && <section className="capture-stream-client"><h4>Client → server</h4><pre>{clientText}</pre></section>}
        {direction !== "client" && <section className="capture-stream-server"><h4>Server → client</h4><pre>{serverText}</pre></section>}
      </div>
      <footer>{packets.length} packets · {byteCount} bytes</footer>
    </section>
  </div>;
}

type StreamChunk = { sequence: number; bytes: Uint8Array };

function streamDirection(packets: PacketRecord[], clientDirection: boolean): Uint8Array {
  if (!packets.length) return new Uint8Array();
  const firstTransport = packets[0].decoded.layers.find((layer) => layer.name.includes("Transmission Control") || layer.name.includes("User Datagram"));
  const firstSourcePort = firstTransport?.fields.find((field) => field.label === "Source Port")?.value;
  const clientEndpoint = `${packets[0].decoded.src}:${firstSourcePort}`;
  const chunks: StreamChunk[] = [];
  for (const packet of packets) {
    const tcp = packet.decoded.layers.find((layer) => layer.name.includes("Transmission Control"));
    const udp = packet.decoded.layers.find((layer) => layer.name.includes("User Datagram"));
    const transport = tcp ?? udp;
    if (!transport) continue;
    const sourcePort = transport.fields.find((field) => field.label === "Source Port")?.value;
    const isClient = `${packet.decoded.src}:${sourcePort}` === clientEndpoint;
    if (isClient !== clientDirection) continue;
    const portOffset = transport.fields.find((field) => field.label === "Source Port")?.byteOffset ?? 0;
    const headerLength = tcp ? Number(transport.fields.find((field) => field.label === "Header Length")?.value.split(" ")[0] ?? 20) : 8;
    const payloadOffset = portOffset + headerLength;
    const sequence = Number(transport.fields.find((field) => field.label === "Sequence Number")?.value ?? packet.number);
    if (payloadOffset < packet.data.length) chunks.push({ sequence, bytes: packet.data.slice(payloadOffset) });
  }
  if (packets[0].decoded.protocol === "TCP" && chunks.length) {
    const base = chunks[0].sequence;
    chunks.sort((left, right) => ((left.sequence - base) >>> 0) - ((right.sequence - base) >>> 0));
  }
  const length = chunks.reduce((total, chunk) => total + chunk.bytes.length, 0);
  const result = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { result.set(chunk.bytes, offset); offset += chunk.bytes.length; }
  return result;
}

function renderStream(bytes: Uint8Array, format: "ascii" | "hex" | "raw"): string {
  if (format === "hex") return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join(" ");
  if (format === "raw") return Array.from(bytes, (value) => String.fromCharCode(value)).join("");
  return new TextDecoder().decode(bytes);
}
