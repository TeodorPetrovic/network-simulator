import { Cable, CircleAlert, Lock, TriangleAlert } from "lucide-react";
import type { TopologySnapshot } from "./api";
import type { DrawingTool } from "./map/helpers";
import { Button } from "@/components/ui/button";

type Props = { topology: TopologySnapshot; projectLocked: boolean; connecting: boolean; linkEndpointCount: number; pendingDrawing: { tool: DrawingTool } | null; onCancelDrawing: () => void; connection: string; onOpenLog: () => void; errors: number; warnings: number; resources: React.ReactNode };

export function ProjectStatusBar({ topology, projectLocked, connecting, linkEndpointCount, pendingDrawing, onCancelDrawing, connection, onOpenLog, errors, warnings, resources }: Props) {
  const connectionLabel = connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable";
  return <footer className="sim-statusbar" aria-label="Project status">
    <div className="sim-statusbar-left"><span>{topology.nodes.length} devices</span><span className="sim-status-separator">·</span><span>{topology.links.length} links</span>{projectLocked && <><span className="sim-status-separator">·</span><Lock size={12} />Locked</>}<span className="sim-status-separator">·</span><span className={`sim-connection-dot ${connection}`} /><span>{connectionLabel}</span>
      {connecting && <span className="sim-status-context"><Cable size={13} />{linkEndpointCount ? "Select second endpoint" : "Select first endpoint"} · Esc to cancel</span>}
      {pendingDrawing && <span className="sim-status-context">Click map to place {pendingDrawing.tool}<Button size="xs" variant="ghost" onClick={onCancelDrawing}>Cancel</Button></span>}
    </div>
    <div className="sim-statusbar-right"><Button variant="ghost" size="sm" className="sim-log-counts" onClick={onOpenLog} aria-label={`Open project log: ${errors} errors, ${warnings} warnings`}><CircleAlert />{errors} errors<TriangleAlert />{warnings} warnings</Button>{resources}</div>
  </footer>;
}
