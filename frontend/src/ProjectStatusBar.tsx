import { Cable, CircleAlert, Lock, TriangleAlert } from "lucide-react";
import type { TopologySnapshot } from "./api";
import type { DrawingTool } from "./map/helpers";
import { Button } from "@/components/ui/button";

type Props = { topology: TopologySnapshot; projectLocked: boolean; connecting: boolean; linkEndpointCount: number; pendingDrawing: { tool: DrawingTool } | null; onCancelDrawing: () => void; connection: string; onOpenLog: () => void; errors: number; warnings: number; resources: React.ReactNode };

export function ProjectStatusBar({ topology, projectLocked, connecting, linkEndpointCount, pendingDrawing, onCancelDrawing, connection, onOpenLog, errors, warnings, resources }: Props) {
  const connectionLabel = connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable";
  return <footer className="z-30 flex min-h-8 shrink-0 items-center justify-between gap-3 border-t bg-sidebar px-3 text-xs text-sidebar-foreground" aria-label="Project status">
    <div className="flex min-w-0 items-center gap-2"><span>{topology.nodes.length} devices</span><span className="text-muted-foreground">·</span><span>{topology.links.length} links</span>{projectLocked && <><span className="text-muted-foreground">·</span><Lock size={12} />Locked</>}<span className="text-muted-foreground">·</span><span className={`size-2 rounded-full ${connection === "connected" ? "bg-success" : connection === "reconnecting" || connection === "checking" ? "bg-warning" : "bg-destructive"}`} /><span>{connectionLabel}</span>
      {connecting && <span className="flex items-center gap-1 text-muted-foreground"><Cable size={13} />{linkEndpointCount ? "Select second endpoint" : "Select first endpoint"} · Esc to cancel</span>}
      {pendingDrawing && <span className="flex items-center gap-1 text-muted-foreground">Click map to place {pendingDrawing.tool}<Button size="xs" variant="ghost" onClick={onCancelDrawing}>Cancel</Button></span>}
    </div>
    <div className="flex shrink-0 items-center gap-1"><Button variant="ghost" size="sm" onClick={onOpenLog} aria-label={`Open project log: ${errors} errors, ${warnings} warnings`}><CircleAlert />{errors} errors<TriangleAlert />{warnings} warnings</Button>{resources}</div>
  </footer>;
}
