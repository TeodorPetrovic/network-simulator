import { useEffect, useState } from "react";
import { CheckCircle2, Download, PackageCheck, RefreshCw, XCircle } from "lucide-react";
import type { RuntimeInfo } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

type SoftwareRow = { key: string; label: string; available: boolean; version: string; detail: string; error?: string | null };
const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

export function InstalledSoftwarePage() {
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  async function refresh() {
    setBusy(true); setError("");
    try { setRuntime(await api.runtimes()); }
    catch (reason) { setError(`Installed software status unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);

  const rows: SoftwareRow[] = runtime ? [
    { key: "docker", label: "Docker Engine", available: runtime.docker.available, version: runtime.docker.version ?? "Not detected", detail: "Container runtime for Docker appliances.", error: runtime.docker.error },
    { key: "qemu", label: "QEMU", available: runtime.qemu.available, version: runtime.qemu.version ?? "Not detected", detail: `${runtime.qemu.image_count} disk images · acceleration: ${runtime.qemu.acceleration}`, error: runtime.qemu.error },
    { key: "qemu-img", label: "qemu-img", available: runtime.qemu.qemu_img_available, version: runtime.qemu.qemu_img_available ? "Available" : "Not detected", detail: `QEMU image directory: ${runtime.qemu.image_directory}`, error: runtime.qemu.qemu_img_available ? null : runtime.qemu.error },
    { key: "vpcs", label: "VPCS", available: runtime.vpcs.available, version: runtime.vpcs.version ?? "Not detected", detail: "Virtual PC simulator runtime.", error: runtime.vpcs.error },
    { key: "dynamips", label: "Dynamips", available: Boolean(runtime.dynamips.version), version: runtime.dynamips.version ?? "Not detected", detail: `${runtime.dynamips.image_count} IOS images · ${runtime.dynamips.image_directory}`, error: runtime.dynamips.error },
    { key: "ubridge", label: "uBridge", available: runtime.ubridge.available, version: runtime.ubridge.version ?? runtime.ubridge.binary, detail: "Ethernet link and NIO bridge service.", error: runtime.ubridge.error },
  ] : [];

  return <PagesShell active="Installed software"><div className="pages-content">
    <PageTitle eyebrow="LOCAL HOST" title="Installed software" description="Runtime binaries detected by the controller and their current availability." actions={<Button size="sm" variant="outline" onClick={() => void refresh()} disabled={busy}><RefreshCw size={15} />Refresh</Button>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}
    {busy && !runtime ? <Card className="pages-empty-state">Checking installed software…</Card> : rows.length ? <div className="software-list">{rows.map((item) => <Card className="software-row" key={item.key}><div className="software-row-icon">{item.available ? <CheckCircle2 size={19} /> : <XCircle size={19} />}</div><div className="software-row-main"><div className="software-row-title"><h2>{item.label}</h2><Badge variant={item.available ? "default" : "secondary"}>{item.available ? "Installed" : "Unavailable"}</Badge></div><p>{item.detail}</p><small>{item.version}{!item.available && item.error ? ` · ${item.error}` : ""}</small></div><Button variant="outline" size="sm" disabled title="Software installation is not exposed by this controller"><Download size={14} />Install unavailable</Button></Card>)}</div> : <Card className="pages-empty-state"><PackageCheck size={18} />No runtime software information was returned by the controller.</Card>}
    <p className="software-note">Installation and upgrade actions are not exposed by the current controller API. This page reports runtime status and does not make local system changes.</p>
  </div></PagesShell>;
}
