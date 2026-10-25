import { Cable, Lock, Pencil } from "lucide-react";
import type { TopologySnapshot } from "./api";
import type { DrawingTool } from "./map/helpers";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

type Props = { topology: TopologySnapshot; projectLocked: boolean; connecting: boolean; linkEndpointCount: number; pendingDrawing: { tool: DrawingTool } | null; onCancelDrawing: () => void };

export function ProjectStatusBar({ topology, projectLocked, connecting, linkEndpointCount, pendingDrawing, onCancelDrawing }: Props) {
  return (
                  <div className="sim-map-status">
                    <div className="sim-map-count"><span className="sim-live-dot" />{topology.nodes.length} devices <span aria-hidden="true">·</span> {topology.links.length} links{projectLocked ? <><span aria-hidden="true">·</span><Lock size={12} />Locked</> : null}</div>
                    {connecting && <div className="sim-link-hint"><Cable size={14} /><span>{linkEndpointCount ? "First end selected. Right-click another device and choose a port. Press Escape to cancel." : "Right-click a device and choose a port. Press Escape to cancel."}</span><Badge variant="secondary">{linkEndpointCount ? "1 of 2" : "0 of 2"}</Badge></div>}
                    {pendingDrawing && <div className="sim-link-hint"><Pencil size={14} /><span>Click to place {pendingDrawing.tool}.</span><Button size="xs" variant="ghost" onClick={() => onCancelDrawing}>Cancel</Button></div>}
                  </div>
  );
}
