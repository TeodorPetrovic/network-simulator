import { Activity, Cpu, HardDrive, RefreshCw, Server } from "lucide-react";
import type { ResourceMonitor, RuntimeInfo } from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";

export function ResourceMonitorPopover({
  resources,
  runtimeInfo,
  error,
  refreshing,
  onRefresh,
  compact = false,
}: {
  resources: ResourceMonitor | null;
  runtimeInfo: RuntimeInfo | null;
  error: string;
  refreshing: boolean;
  onRefresh: () => void;
  compact?: boolean;
}) {
  const cpu = resources?.host.cpu_usage_percent;
  const memoryUsed = resources?.host.memory_used_bytes;
  const memoryTotal = resources?.host.memory_total_bytes;
  const memoryPercent = memoryUsed != null && memoryTotal ? memoryUsed / memoryTotal * 100 : null;
  const engines = runtimeInfo ? [
    ["Docker", runtimeInfo.docker.available],
    ["QEMU / KVM", runtimeInfo.qemu.available],
    ["VPCS", runtimeInfo.vpcs.available],
    ["Dynamips", runtimeInfo.dynamips.available],
    ["uBridge", runtimeInfo.ubridge.available],
  ] as const : [];

  return <Popover>
    <PopoverTrigger asChild><Button variant="outline" size={compact ? "icon-sm" : "sm"} className="sim-resource-trigger" aria-label={compact ? "Resources" : undefined} title={compact ? "Resource monitor" : undefined}><Activity />{!compact && <>Resources{cpu != null && <Badge variant="secondary">{Math.round(cpu)}%</Badge>}</>}</Button></PopoverTrigger>
    <PopoverContent align="end" sideOffset={9} className={`${compact ? "w-[320px]" : "w-[360px]"} p-0`}>
      <div className="flex items-center justify-between px-3 py-2.5">
        <div><p className="text-sm font-semibold">Resource monitor</p><p className="text-xs text-muted-foreground">Live host and controller usage</p></div>
        <Button variant="ghost" size="icon-sm" disabled={refreshing} onClick={onRefresh} aria-label="Refresh resource monitor"><RefreshCw className={refreshing ? "animate-spin" : ""} /></Button>
      </div>
      <Separator />
      <div className="space-y-3 p-3">
        {resources ? <>
          <Metric label="Host CPU" icon={<Cpu />} value={cpu == null ? "Unavailable" : `${cpu.toFixed(1)}%`} progress={cpu} detail={resources.host.cpu_count ? `${resources.host.cpu_count} logical CPUs` : "CPU count unavailable"} />
          <Metric label="Host memory" icon={<HardDrive />} value={memoryPercent == null ? "Unavailable" : `${memoryPercent.toFixed(1)}%`} progress={memoryPercent} detail={memoryUsed == null || memoryTotal == null ? "Memory data unavailable" : `${formatBytes(memoryUsed)} of ${formatBytes(memoryTotal)}`} />
          <Card className="grid grid-cols-2 gap-3 p-3 shadow-none">
            <div><p className="text-xs text-muted-foreground">Controller RSS</p><p className="mt-1 text-sm font-semibold">{resources.controller.memory_rss_bytes == null ? "Unavailable" : formatBytes(resources.controller.memory_rss_bytes)}</p></div>
            <div><p className="text-xs text-muted-foreground">Controller threads</p><p className="mt-1 text-sm font-semibold">{resources.controller.thread_count ?? "Unavailable"}</p></div>
            <div className="col-span-2"><p className="text-xs text-muted-foreground">System load · 1 / 5 / 15 min</p><p className="mt-1 font-mono text-xs">{resources.host.load_average?.map((value) => value.toFixed(2)).join(" · ") ?? "Unavailable"}</p></div>
          </Card>
          {resources.sampled_at_unix_ms && <p className="text-right text-[11px] text-muted-foreground">Sampled {new Date(resources.sampled_at_unix_ms).toLocaleTimeString()}</p>}
        </> : <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">{error || "Collecting host metrics…"}</p>}
        <Separator />
        <div>
          <div className="mb-2 flex items-center gap-2 text-xs font-medium"><Server className="size-4" />Runtime services</div>
          <div className="grid grid-cols-2 gap-2">{engines.map(([name, available]) => <div key={name} className="flex items-center justify-between gap-2 rounded-md border px-2.5 py-2"><span className="truncate text-xs">{name}</span><Badge variant={available ? "default" : "secondary"}>{available ? "Ready" : "Offline"}</Badge></div>)}</div>
        </div>
      </div>
    </PopoverContent>
  </Popover>;
}

function Metric({ label, icon, value, progress, detail }: { label: string; icon: React.ReactNode; value: string; progress: number | null | undefined; detail: string }) {
  return <div>
    <div className="mb-1.5 flex items-center justify-between gap-3"><div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">{icon}{label}</div><span className="text-sm font-semibold tabular-nums">{value}</span></div>
    {progress != null && <Progress value={Math.max(0, Math.min(100, progress))} className="h-1.5" />}
    <p className="mt-1 text-[11px] text-muted-foreground">{detail}</p>
  </div>;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let index = -1;
  do { value /= 1024; index += 1; } while (value >= 1024 && index < units.length - 1);
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`;
}
