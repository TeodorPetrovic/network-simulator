import { useEffect, useState } from "react";
import type { TopologySnapshot } from "./api";
import type { Selection } from "./map/TopologyCanvas";

export function useWorkspaceSelection(selection: Selection, topology: TopologySnapshot) {
  const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>(selection?.kind === "node" ? [selection.id] : []);
  const [selectedDrawingId, setSelectedDrawingId] = useState<string | null>(null);
  const selectedDrawing = topology.drawings.find((drawing) => drawing.drawing_id === selectedDrawingId) ?? null;

  useEffect(() => {
    setSelectedNodeIds(selection?.kind === "node" ? [selection.id] : []);
  }, [selection?.kind, selection?.id]);

  return { selectedNodeIds, setSelectedNodeIds, selectedDrawingId, setSelectedDrawingId, selectedDrawing };
}
