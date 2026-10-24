import { api } from "./api";
import type { Dispatch, SetStateAction } from "react";
import type { Link, Node, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { drawingSvg, reorderDrawings } from "./map/helpers";
import type { DrawingTool } from "./map/helpers";
import { messageFor, portLabel } from "./map/TopologyCanvas";

export type DrawingValues = { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string };
type PendingDrawing = { tool: DrawingTool; values: DrawingValues } | null;
type Drawing = TopologySnapshot["drawings"][number];
type Args = {
  workspace: Pick<SimulatorWorkspaceProps, "project" | "onSelectCanvas">; topology: TopologySnapshot; selectedDrawing: Drawing | null;
  pendingDrawing: PendingDrawing; projectLocked: boolean; setSelectedNodeIds: (ids: string[]) => void; setTopology: Dispatch<SetStateAction<TopologySnapshot>>;
  setSelectedDrawingId: (id: string | null) => void; setPendingDrawing: (drawing: PendingDrawing) => void;
  setFeatureMessage: (message: string) => void; setDrawingEditOpen: (open: boolean) => void; setProjectLocked: (locked: boolean) => void;
};

export function useWorkspaceActions({ workspace: props, topology, selectedDrawing, pendingDrawing, projectLocked, setSelectedNodeIds, setTopology, setSelectedDrawingId, setPendingDrawing, setFeatureMessage, setDrawingEditOpen, setProjectLocked }: Args) {
  async function placeDrawing(point: { x: number; y: number }) {
    if (!pendingDrawing) { setSelectedDrawingId(null); setSelectedNodeIds([]); props.onSelectCanvas(); return; }
    try {
      const { tool, values } = pendingDrawing;
      const drawing = await api.createDrawing(props.project.project_id, {
        x: point.x, y: point.y, width: values.width, height: values.height, z_index: topology.drawings.length,
        drawing_kind: tool, svg: drawingSvg(tool, values),
      });
      setTopology((current) => ({ ...current, drawings: [...current.drawings, drawing] }));
      setSelectedDrawingId(drawing.drawing_id);
      setPendingDrawing(null);
      setFeatureMessage(`${tool[0]?.toUpperCase()}${tool.slice(1)} drawing added.`);
    } catch (reason) { setFeatureMessage(`Could not add drawing: ${messageFor(reason)}`); }
  }

  async function saveEditedDrawing(tool: DrawingTool, values: DrawingValues) {
    if (!selectedDrawing) return;
    try {
      const updated = await api.updateDrawing(props.project.project_id, selectedDrawing.drawing_id, {
        width: values.width, height: values.height, drawing_kind: tool, svg: drawingSvg(tool, values),
      });
      setTopology((current) => ({ ...current, drawings: current.drawings.map((drawing) => drawing.drawing_id === updated.drawing_id ? updated : drawing) }));
      setDrawingEditOpen(false);
    } catch (reason) { setFeatureMessage(`Could not edit drawing: ${messageFor(reason)}`); }
  }

  async function removeDrawing() {
    if (!selectedDrawing) return;
    try {
      await api.deleteDrawing(props.project.project_id, selectedDrawing.drawing_id);
      setTopology((current) => ({ ...current, drawings: current.drawings.filter((drawing) => drawing.drawing_id !== selectedDrawing.drawing_id) }));
      setSelectedDrawingId(null);
    } catch (reason) { setFeatureMessage(`Could not delete drawing: ${messageFor(reason)}`); }
  }

  async function moveDrawing(drawingId: string, x: number, y: number) {
    try {
      const drawing = await api.updateDrawing(props.project.project_id, drawingId, { x, y });
      setTopology((current) => ({ ...current, drawings: current.drawings.map((item) => item.drawing_id === drawingId ? drawing : item) }));
    } catch (reason) { setFeatureMessage(`Could not move drawing: ${messageFor(reason)}`); }
  }

  async function changeDrawingOrder(direction: "front" | "back") {
    if (!selectedDrawing) return;
    const reordered = reorderDrawings(topology.drawings, selectedDrawing.drawing_id, direction);
    try {
      const updated = await Promise.all(reordered.map((drawing, z_index) => api.updateDrawing(props.project.project_id, drawing.drawing_id, { z_index })));
      setTopology((current) => ({ ...current, drawings: current.drawings.map((drawing) => updated.find((item) => item.drawing_id === drawing.drawing_id) ?? drawing) }));
    } catch (reason) { setFeatureMessage(`Could not change drawing order: ${messageFor(reason)}`); }
  }

  async function toggleProjectLock() {
    const locked = !projectLocked;
    try {
      if (locked) await api.lockProject(props.project.project_id);
      else await api.unlockProject(props.project.project_id);
      setProjectLocked(locked); setFeatureMessage(locked ? "Project locked." : "Project unlocked.");
    } catch (reason) { setFeatureMessage(`Project lock is not available: ${messageFor(reason)}`); }
  }

  async function toggleNodeLock(node: Node) {
    const locked = !Boolean(node.locked);
    try {
      const updated = await api.updateNode(props.project.project_id, node.node_id, { locked });
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === node.node_id ? updated : item) }));
      setFeatureMessage(locked ? `${node.name} locked.` : `${node.name} unlocked.`);
    } catch (reason) { setFeatureMessage(`Node lock is not available: ${messageFor(reason)}`); }
  }

  async function toggleNodeIsolation(node: Node) {
    try {
      const updated = node.isolated ? await api.unisolateNode(props.project.project_id, node.node_id) : await api.isolateNode(props.project.project_id, node.node_id);
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === node.node_id ? updated : item) }));
      setFeatureMessage(updated.isolated ? `${node.name} isolated.` : `${node.name} unisolated.`);
    } catch (reason) { setFeatureMessage(`Node isolation is not available: ${messageFor(reason)}`); }
  }

  async function toggleLinkSuspend(link: Link) {
    try {
      const updated = await api.updateLink(props.project.project_id, link.link_id, { suspend: !link.suspend });
      setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === link.link_id ? updated : item) }));
      setFeatureMessage(updated.suspend ? "Link suspended." : "Link resumed.");
    } catch (reason) { setFeatureMessage(`Link suspend is not available: ${messageFor(reason)}`); }
  }

  async function saveNodeLabelPosition(nodeId: string, x: number, y: number) {
    const node = topology.nodes.find((item) => item.node_id === nodeId);
    if (!node) return;
    const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
    try {
      const updated = await api.updateNode(props.project.project_id, nodeId, { label: { ...label, x, y } });
      setTopology((current) => ({ ...current, nodes: current.nodes.map((item) => item.node_id === nodeId ? updated : item) }));
    } catch (reason) { setFeatureMessage(`Could not save label position: ${messageFor(reason)}`); }
  }

  async function saveLinkEndpointLabelPosition(linkId: string, nodeId: string, x: number, y: number) {
    const link = topology.links.find((item) => item.link_id === linkId);
    const node = topology.nodes.find((item) => item.node_id === nodeId);
    if (!link || !node) return;
    try {
      const updated = await api.updateLink(props.project.project_id, linkId, {
        nodes: link.nodes.map((endpoint) => endpoint.node_id === nodeId
          ? { ...endpoint, label: { ...endpoint.label, text: endpoint.label?.text ?? portLabel(node, endpoint), x, y } }
          : endpoint),
      });
      setTopology((current) => ({ ...current, links: current.links.map((item) => item.link_id === updated.link_id ? updated : item) }));
    } catch (reason) { setFeatureMessage(`Could not save endpoint label position: ${messageFor(reason)}`); }
  }

  return { placeDrawing, saveEditedDrawing, removeDrawing, moveDrawing, changeDrawingOrder, toggleProjectLock, toggleNodeLock, toggleNodeIsolation, toggleLinkSuspend, saveNodeLabelPosition, saveLinkEndpointLabelPosition };
}
