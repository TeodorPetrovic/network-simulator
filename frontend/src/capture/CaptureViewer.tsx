import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { dissect } from "./dissect";
import { readPcap } from "./pcap";
import { compileDisplayFilter, DisplayFilterError, type PacketRecord } from "./filter";
import { CaptureFilterBar, CaptureStatusBar, CaptureToolbar, FollowStreamDialog, PacketDetails, PacketList, StatisticsPanel } from "./CaptureComponents";
import "./capture.css";

export type CaptureViewerProps = {
  streamUrl?: string;
  downloadUrl?: string;
  title?: string;
  onClose?: () => void;
};

type Tab = "packets" | "statistics";

export function CaptureViewer({ streamUrl, downloadUrl, title = "Capture Viewer" }: CaptureViewerProps) {
  const [packets, setPackets] = useState<PacketRecord[]>([]);
  const [filter, setFilter] = useState("");
  const [paused, setPaused] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [relativeTime, setRelativeTime] = useState(true);
  const [selectedNumber, setSelectedNumber] = useState<number | null>(null);
  const [tab, setTab] = useState<Tab>("packets");
  const [status, setStatus] = useState("Ready");
  const [followPackets, setFollowPackets] = useState<PacketRecord[] | null>(null);
  const [paneHeight, setPaneHeight] = useState(42);
  const fileInput = useRef<HTMLInputElement>(null);
  const filterInput = useRef<HTMLInputElement>(null);
  const nextNumber = useRef(1);
  const pausedRef = useRef(paused);
  const pausedWaiters = useRef<Array<() => void>>([]);
  const abort = useRef<AbortController | null>(null);

  useEffect(() => { pausedRef.current = paused; }, [paused]);

  const appendCapture = useCallback(async (body: ReadableStream<Uint8Array>, label: string) => {
    const batch: PacketRecord[] = [];
    setStatus(label);
    try {
      for await (const raw of readPcap(body)) {
        while (pausedRef.current) await new Promise<void>((resolve) => pausedWaiters.current.push(resolve));
        batch.push({
          ...raw,
          number: nextNumber.current++,
          time: raw.timestamp,
          decoded: dissect(raw.data, raw.linkType),
        });
        if (batch.length >= 128) {
          const added = batch.splice(0);
          await new Promise<void>((resolve) => {
            const commit = () => { setPackets((current) => [...current, ...added]); resolve(); };
            const idleWindow = window as Window & { requestIdleCallback?: (callback: () => void, options?: { timeout: number }) => number };
            if (idleWindow.requestIdleCallback) idleWindow.requestIdleCallback(commit, { timeout: 100 });
            else window.setTimeout(commit, 0);
          });
        }
      }
      if (batch.length) setPackets((current) => [...current, ...batch]);
      setStatus(`${nextNumber.current - 1} packets · ${label}`);
    } catch (reason) {
      if (reason instanceof Error && reason.name === "AbortError") return;
      setStatus(reason instanceof Error ? reason.message : "Capture stream failed");
    }
  }, []);

  useEffect(() => {
    const sourceUrl = streamUrl ?? downloadUrl;
    if (!sourceUrl) return;
    const controller = new AbortController();
    abort.current = controller;
    void fetch(sourceUrl, { signal: controller.signal }).then((response) => {
      if (!response.ok || !response.body) throw new Error(`Capture request failed (${response.status})`);
      return appendCapture(response.body, streamUrl ? "Capturing" : "Saved capture");
    }).catch((reason: unknown) => {
      if (reason instanceof Error && reason.name !== "AbortError") setStatus(reason.message);
    });
    return () => controller.abort();
  }, [appendCapture, downloadUrl, streamUrl]);

  const filtered = useMemo(() => {
    if (!filter.trim()) return { packets, error: "" };
    try {
      const predicate = compileDisplayFilter(filter);
      return { packets: packets.filter(predicate), error: "" };
    } catch (reason) {
      return { packets: [] as PacketRecord[], error: reason instanceof DisplayFilterError ? reason.message : "Invalid display filter" };
    }
  }, [filter, packets]);
  const selected = packets.find((packet) => packet.number === selectedNumber) ?? filtered.packets[0];

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "f") {
        event.preventDefault();
        filterInput.current?.focus();
        return;
      }
      if (event.target instanceof HTMLInputElement || !filtered.packets.length) return;
      const current = filtered.packets.findIndex((packet) => packet.number === selectedNumber);
      let next = current;
      if (event.key === "ArrowDown") next = Math.min(filtered.packets.length - 1, Math.max(0, current + 1));
      else if (event.key === "ArrowUp") next = Math.max(0, current < 0 ? 0 : current - 1);
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = filtered.packets.length - 1;
      else return;
      event.preventDefault();
      setSelectedNumber(filtered.packets[next].number);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [filtered.packets, selectedNumber]);

  function clearCapture() {
    setPackets([]);
    setSelectedNumber(null);
    nextNumber.current = 1;
    setStatus("Cleared");
  }

  async function loadFile(file: File) {
    clearCapture();
    setStatus(`Reading ${file.name}…`);
    const bytes = new Uint8Array(await file.arrayBuffer());
    const stream = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(bytes); controller.close(); } });
    await appendCapture(stream, file.name);
  }

  async function openDownloadedCapture() {
    if (!downloadUrl) return;
    setStatus("Downloading saved capture…");
    try {
      const response = await fetch(downloadUrl);
      if (!response.ok || !response.body) throw new Error(`Download failed (${response.status})`);
      clearCapture();
      await appendCapture(response.body, "Saved capture");
    } catch (reason) {
      setStatus(reason instanceof Error ? reason.message : "Could not download capture");
    }
  }

  function saveCapture() {
    const size = 24 + packets.reduce((total, packet) => total + 16 + packet.data.length, 0);
    const bytes = new Uint8Array(size);
    bytes.set([0xd4, 0xc3, 0xb2, 0xa1]);
    const data = new DataView(bytes.buffer);
    data.setUint16(4, 2, true);
    data.setUint16(6, 4, true);
    data.setUint32(16, 262144, true);
    data.setUint32(20, packets[0]?.linkType ?? 1, true);
    let offset = 24;
    for (const packet of packets) {
      const seconds = Math.floor(packet.timestamp);
      data.setUint32(offset, seconds, true);
      data.setUint32(offset + 4, Math.round((packet.timestamp - seconds) * 1e6), true);
      data.setUint32(offset + 8, packet.data.length, true);
      data.setUint32(offset + 12, packet.originalLength, true);
      bytes.set(packet.data, offset + 16);
      offset += 16 + packet.data.length;
    }
    const url = URL.createObjectURL(new Blob([bytes], { type: "application/vnd.tcpdump.pcap" }));
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "capture.pcap";
    anchor.click();
    URL.revokeObjectURL(url);
  }

  function followSelectedStream() {
    if (!selected) return;
    const transport = selected.decoded.layers.find((layer) => layer.name.includes("Transmission Control") || layer.name.includes("User Datagram"));
    if (!transport) return;
    const ports = transport.fields.filter((field) => field.label.endsWith("Port")).map((field) => field.value);
    const peer = packets.filter((packet) => {
      if (packet.decoded.protocol !== selected.decoded.protocol || packet.decoded.src !== selected.decoded.src && packet.decoded.dst !== selected.decoded.src) return false;
      const currentTransport = packet.decoded.layers.find((layer) => layer.name.includes("Transmission Control") || layer.name.includes("User Datagram"));
      const currentPorts = currentTransport?.fields.filter((field) => field.label.endsWith("Port")).map((field) => field.value) ?? [];
      return currentPorts.length === 2 && currentPorts.includes(ports[0]) && currentPorts.includes(ports[1]);
    });
    setFollowPackets(peer);
  }

  return <div className="capture-viewer" onKeyDown={(event) => {
    if (event.key === "f" && event.altKey) followSelectedStream();
  }}>
    <CaptureToolbar
      title={title}
      paused={paused}
      autoScroll={autoScroll}
      relativeTime={relativeTime}
      hasDownload={Boolean(downloadUrl)}
      fileInput={fileInput}
      onToggleTime={() => setRelativeTime((value) => !value)}
      onToggleScroll={() => setAutoScroll((value) => !value)}
      onTogglePause={() => {
        const nextPaused = !pausedRef.current;
        pausedRef.current = nextPaused;
        setPaused(nextPaused);
        if (!nextPaused) pausedWaiters.current.splice(0).forEach((resolve) => resolve());
      }}
      onFollow={followSelectedStream}
      onClear={clearCapture}
      onSave={saveCapture}
      onOpenSaved={() => void openDownloadedCapture()}
    />
      <input ref={fileInput} hidden type="file" accept=".pcap,.pcapng,application/vnd.tcpdump.pcap" onChange={(event) => {
        const file = event.target.files?.[0];
        if (file) void loadFile(file);
        event.target.value = "";
      }} />
    <CaptureFilterBar value={filter} error={filtered.error} onChange={setFilter} inputRef={filterInput} />
    <div className="capture-tabs">
      <button onClick={() => setTab("packets")} aria-selected={tab === "packets"}>Packets</button>
      <button onClick={() => setTab("statistics")} aria-selected={tab === "statistics"}>Statistics</button>
    </div>
    {tab === "packets" ? <>
      <div className="capture-table" style={{ height: `${paneHeight}%` }}>
        <PacketList packets={filtered.packets} selectedNumber={selected?.number ?? null} relativeTime={relativeTime} autoScroll={autoScroll} onSelect={(packet) => setSelectedNumber(packet.number)} onApplyFilter={setFilter} />
      </div>
      <div className="capture-splitter" role="separator" aria-orientation="horizontal" onPointerDown={(event) => {
        const startY = event.clientY;
        const initial = paneHeight;
        const move = (pointer: PointerEvent) => setPaneHeight(Math.max(20, Math.min(75, initial + ((pointer.clientY - startY) / Math.max(1, innerHeight)) * 100)));
        const end = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", end);
      }} />
      <PacketDetails packet={selected} onApplyFilter={setFilter} />
    </> : <StatisticsPanel packets={filtered.packets} />}
    <CaptureStatusBar status={status} displayed={filtered.packets.length} total={packets.length} />
    {followPackets && <FollowStreamDialog packets={followPackets} onClose={() => setFollowPackets(null)} />}
  </div>;
}
