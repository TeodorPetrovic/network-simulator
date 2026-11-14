import { useState } from "react";
import { Activity, CircleAlert, CircleCheck, CircleHelp, Pause, Power, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { HealthTone, ProjectHealth } from "./projectHealth";

export const healthColors: Record<HealthTone, string> = {
  success: "border-success/30 bg-success/10 text-success",
  warning: "border-warning/40 bg-warning/10 text-amber-700 dark:text-amber-400",
  danger: "border-destructive/30 bg-destructive/10 text-destructive",
  neutral: "border-border bg-muted/50 text-muted-foreground",
};

export function ProjectHealthIndicator({ health, onInspectNode, checkedAt, stale = false }: { health: ProjectHealth; onInspectNode?: (id: string) => void; checkedAt?: number; stale?: boolean }) {
  const [open, setOpen] = useState(false);
  const Icon = health.tone === "danger" ? CircleAlert : health.tone === "warning" ? TriangleAlert : health.tone === "success" ? CircleCheck : Power;
  return <Popover open={open} onOpenChange={setOpen}>
    <PopoverTrigger asChild><Button variant="ghost" size="sm" className={`h-7 gap-1.5 border px-2 text-xs ${healthColors[health.tone]}`} aria-label={`Project health: ${health.label}`}><Icon className="size-3.5" />{health.label}</Button></PopoverTrigger>
    <PopoverContent side="top" align="start" className="max-h-[min(36rem,80dvh)] w-[min(25rem,calc(100vw-2rem))] gap-0 overflow-y-auto p-0">
      <div className="space-y-2 border-b p-4"><h3 className="flex items-center gap-2 font-semibold"><Activity className="size-4" />Project health</h3><p className="text-xs leading-relaxed text-muted-foreground">{health.summary}</p></div>
      <div className="p-4">
        <p className="mb-2 text-xs font-medium">{stale ? "Last known device states" : "Device states"}</p>
        <dl className="grid grid-cols-2 gap-2 text-xs">
          {([ ["running", "Running", CircleCheck], ["stopped", "Stopped", Power], ["suspended", "Suspended", Pause], ["unavailable", "Unavailable", CircleAlert], ["unknown", "Unknown", CircleHelp] ] as const).map(([key, label, StateIcon]) => <div key={key} className="flex items-center justify-between gap-2 rounded-md border p-2"><dt className="flex items-center gap-1.5 text-muted-foreground"><StateIcon className="size-3.5" />{label}</dt><dd className="font-semibold tabular-nums">{health.counts[key]}</dd></div>)}
        </dl>
        {health.issues.length > 0 && <div className="mt-4 space-y-2"><h4 className="text-xs font-medium">Items to review · {health.issues.length}</h4>{health.issues.map((issue) => <div key={issue.id} className="space-y-1.5 rounded-lg border p-3"><p className="flex items-start gap-2 text-xs font-semibold">{issue.tone === "danger" ? <CircleAlert className="size-3.5 shrink-0 text-destructive" /> : <TriangleAlert className="size-3.5 shrink-0 text-amber-600" />}{issue.title}</p><p className="text-xs leading-relaxed text-muted-foreground">{issue.detail}</p>{issue.nodeId && onInspectNode && <Button variant="outline" size="xs" onClick={() => { onInspectNode(issue.nodeId!); setOpen(false); }}>Inspect {issue.title}</Button>}</div>)}</div>}
      </div>
      <div className="space-y-1 border-t bg-muted/30 px-4 py-3 text-[11px] leading-relaxed text-muted-foreground">
        {checkedAt && <p>Checked at {new Date(checkedAt).toLocaleTimeString()}</p>}
        <p>Based on reported device and link states. Guest connectivity and routing have not been tested.</p>
      </div>
    </PopoverContent>
  </Popover>;
}
