import { useEffect, useMemo, useRef, useState } from "react";
import type { DragEvent, PointerEvent } from "react";
import { Network } from "lucide-react";
import type { Drawing, Link, LinkEndpoint, Node } from "../api";
import { api } from "../api";
import { sanitizeDrawingSvg } from "../topology";
import { selectionBoxIds } from "./helpers";
import { useCanvasKeyboardShortcuts } from "./useCanvasKeyboardShortcuts";
import { resolveCanvasSymbol } from "./symbols";
import { iconForSymbolName, iconForType, localPortPoint, portCount, portIndex, portLabel, portPoint } from "./topologyUtils";
export { deviceGroupLabel, endpointLabel, iconForSymbolName, iconForType, imageName, kindLabel, localPortPoint, messageFor, nodeHasAuxConsole, nodePorts, portCount, portIndex, portLabel, portPoint, safeFileName, sameEndpoint, templateDescription, valuesFromDrawing } from "./topologyUtils";

export const scene = { width: 1440, height: 920 };
export const maxZoom = 3;

export type Selection = { kind: "node" | "link"; id: string } | null;
type CanvasProps = {
  nodes: Node[];
  links: Link[];
  drawings: Drawing[];
  symbolDefaults: Record<string, string>;
  customSymbolIds: string[];
  zoom: number;
  centerX: number;
  centerY: number;
  gridVisible: boolean;
  snapToGrid: boolean;
  showInterfaceLabels?: boolean;
  onToggleInterfaceLabels?: () => void;
  projectLocked: boolean;
  selectedNodeIds: string[];
  selectedDrawingId: string | null;
  onCenterChange: (x: number, y: number) => void;
  selection: Selection;
  connecting: boolean;
  linkEndpoints: LinkEndpoint[];
  onNodeClick: (id: string) => void;
  onMultiSelectNode: (id: string) => void;
  onMultiSelect: (ids: string[]) => void;
  onLinkClick: (id: string) => void;
  onCanvasClick: (point: { x: number; y: number }) => void;
  onPortClick: (endpoint: LinkEndpoint) => void;
  onNodeContext: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
  onNodeLabelMove: (id: string, x: number, y: number) => void;
  onLinkEndpointLabelMove: (linkId: string, nodeId: string, x: number, y: number) => void;
  onDrawingClick: (id: string) => void;
  onDrawingMove: (id: string, x: number, y: number) => void;
  onDeleteSelected: () => void;
  onClearSelection: () => void;
  onZoom: (delta: number) => void;
  onDropTemplate: (templateId: string, x: number, y: number) => void;
};

type DragState = { nodeIds: string[]; pointerId: number; start: { x: number; y: number }; origins: Map<string, { x: number; y: number }> };
type LabelDragState = { nodeId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type LinkLabelDragState = { linkId: string; nodeId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type DrawingDragState = { drawingId: string; pointerId: number; start: { x: number; y: number }; origin: { x: number; y: number } };
type PanState = { pointerId: number; startX: number; startY: number; centerX: number; centerY: number };

export function TopologyCanvas(props: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const labelDragRef = useRef<LabelDragState | null>(null);
  const linkLabelDragRef = useRef<LinkLabelDragState | null>(null);
  const drawingDragRef = useRef<DrawingDragState | null>(null);
  const selectionRef = useRef<{ pointerId: number; start: { x: number; y: number }; additive: boolean } | null>(null);
  const panRef = useRef<PanState | null>(null);
  const panMovedRef = useRef(false);
  const movedRef = useRef(false);
  const lastMoveRef = useRef<Map<string, { x: number; y: number }> | null>(null);
  const { spaceDownRef, onKeyDown, onKeyUp, onBlur } = useCanvasKeyboardShortcuts(props, lastMoveRef);
  const [dropActive, setDropActive] = useState(false);
  const [selectionBox, setSelectionBox] = useState<{ x: number; y: number; width: number; height: number } | null>(null);
  const [drawingPositions, setDrawingPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [labelPositions, setLabelPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [linkLabelPositions, setLinkLabelPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [cursorPoint, setCursorPoint] = useState<{ x: number; y: number } | null>(null);
  const view = { width: scene.width / props.zoom, height: scene.height / props.zoom, x: props.centerX - scene.width / props.zoom / 2, y: props.centerY - scene.height / props.zoom / 2 };
  const nodeMap = useMemo(() => new Map(props.nodes.map((node) => [node.node_id, node])), [props.nodes]);
  const renderedDrawings = useMemo(() => props.drawings.flatMap((drawing) => {
    const markup = typeof drawing.svg === "string" ? sanitizeDrawingSvg(drawing.svg) : "";
    return markup ? [{ drawing, markup }] : [];
  }), [props.drawings]);
  const orderedDrawings = useMemo(() => [...renderedDrawings].sort((left, right) => Number(left.drawing.z_index ?? 0) - Number(right.drawing.z_index ?? 0)), [renderedDrawings]);

  const scenePoint = (clientX: number, clientY: number, rect: DOMRect) => {
    const scale = Math.min(rect.width / view.width, rect.height / view.height);
    const offsetX = (rect.width - view.width * scale) / 2;
    const offsetY = (rect.height - view.height * scale) / 2;
    const x = view.x + (clientX - rect.left - offsetX) / scale;
    const y = view.y + (clientY - rect.top - offsetY) / scale;
    return props.snapToGrid ? { x: Math.round(x / 10) * 10, y: Math.round(y / 10) * 10 } : { x: Math.round(x), y: Math.round(y) };
  };

  function startMove(event: PointerEvent<SVGGElement>, node: Node) {
    if (event.button === 2) {
      props.onNodeContext(node.node_id);
      return;
    }
    if (props.connecting || props.projectLocked || node.locked || event.button !== 0 || (event.target as Element).closest("[data-interface-port], [data-node-label]")) return;
    if (event.metaKey || event.ctrlKey || !svgRef.current) return;
    event.stopPropagation();
    const nodeIds = props.selectedNodeIds.includes(node.node_id) ? props.selectedNodeIds : [node.node_id];
    const origins = new Map(nodeIds.map((id) => { const item = nodeMap.get(id); return [id, { x: Number(item?.x ?? 0), y: Number(item?.y ?? 0) }] as const; }));
    dragRef.current = { nodeIds, pointerId: event.pointerId, start: scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect()), origins };
    if (!props.selectedNodeIds.includes(node.node_id)) props.onMultiSelect(nodeIds);
    if (nodeIds.length === 1 && (props.selection?.kind !== "node" || props.selection.id !== node.node_id)) props.onNodeClick(node.node_id);
    movedRef.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startLabelMove(event: PointerEvent<SVGTextElement>, node: Node) {
    if (props.projectLocked || node.locked || event.button !== 0 || props.connecting || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
    labelDragRef.current = { nodeId: node.node_id, pointerId: event.pointerId, start: point, origin: { x: Number(label.x ?? 0), y: Number(label.y ?? 52) } };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startLinkLabelMove(event: PointerEvent<SVGTextElement>, link: Link, endpoint: LinkEndpoint, node: Node) {
    if (props.projectLocked || node.locked || event.button !== 0 || props.connecting || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    const label = endpoint.label;
    const port = localPortPoint(portIndex(node, endpoint), portCount(node));
    linkLabelDragRef.current = {
      linkId: link.link_id,
      nodeId: endpoint.node_id,
      pointerId: event.pointerId,
      start: point,
      origin: { x: Number(label?.x ?? port.x + 8), y: Number(label?.y ?? port.y + 4) },
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startDrawingMove(event: PointerEvent<SVGGElement>, drawing: Drawing) {
    if (props.projectLocked || event.button !== 0 || !svgRef.current) return;
    event.stopPropagation();
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    drawingDragRef.current = { drawingId: drawing.drawing_id, pointerId: event.pointerId, start: point, origin: { x: Number(drawing.x ?? 0), y: Number(drawing.y ?? 0) } };
    props.onDrawingClick(drawing.drawing_id);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function startPan(event: PointerEvent<SVGSVGElement>) {
    const isBackground = event.target === event.currentTarget || (event.target as Element).tagName === "rect";
    const panButton = event.button === 1 || (event.button === 0 && spaceDownRef.current);
    if (!isBackground || (event.button !== 0 && event.button !== 1) || !svgRef.current) return;
    if (panButton) {
      event.preventDefault();
      panRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, centerX: props.centerX, centerY: props.centerY };
      panMovedRef.current = false;
      svgRef.current.setPointerCapture(event.pointerId);
      return;
    }
    const start = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    selectionRef.current = { pointerId: event.pointerId, start, additive: event.shiftKey || event.metaKey || event.ctrlKey };
    setSelectionBox({ x: start.x, y: start.y, width: 0, height: 0 });
    panMovedRef.current = false;
    svgRef.current.setPointerCapture(event.pointerId);
  }

  function moveNode(event: PointerEvent<SVGSVGElement>) {
    if (props.connecting && props.linkEndpoints.length === 1 && svgRef.current) setCursorPoint(scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect()));
    const selection = selectionRef.current;
    if (selection && selection.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      if (Math.abs(point.x - selection.start.x) + Math.abs(point.y - selection.start.y) > 2) panMovedRef.current = true;
      setSelectionBox({ x: Math.min(selection.start.x, point.x), y: Math.min(selection.start.y, point.y), width: Math.abs(point.x - selection.start.x), height: Math.abs(point.y - selection.start.y) });
      return;
    }
    const pan = panRef.current;
    if (pan && pan.pointerId === event.pointerId && svgRef.current) {
      const rect = svgRef.current.getBoundingClientRect();
      const scale = Math.min(rect.width / view.width, rect.height / view.height);
      const dx = (event.clientX - pan.startX) / scale;
      const dy = (event.clientY - pan.startY) / scale;
      if (Math.abs(dx) + Math.abs(dy) > 2) panMovedRef.current = true;
      props.onCenterChange(pan.centerX - dx, pan.centerY - dy);
      return;
    }
    const linkLabelDrag = linkLabelDragRef.current;
    if (linkLabelDrag && linkLabelDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      const key = `${linkLabelDrag.linkId}:${linkLabelDrag.nodeId}`;
      setLinkLabelPositions((current) => ({ ...current, [key]: { x: linkLabelDrag.origin.x + point.x - linkLabelDrag.start.x, y: linkLabelDrag.origin.y + point.y - linkLabelDrag.start.y } }));
      return;
    }
    const labelDrag = labelDragRef.current;
    if (labelDrag && labelDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      setLabelPositions((current) => ({ ...current, [labelDrag.nodeId]: { x: labelDrag.origin.x + point.x - labelDrag.start.x, y: labelDrag.origin.y + point.y - labelDrag.start.y } }));
      return;
    }
    const drawingDrag = drawingDragRef.current;
    if (drawingDrag && drawingDrag.pointerId === event.pointerId && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      movedRef.current = true;
      setDrawingPositions((current) => ({ ...current, [drawingDrag.drawingId]: { x: drawingDrag.origin.x + point.x - drawingDrag.start.x, y: drawingDrag.origin.y + point.y - drawingDrag.start.y } }));
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId || !svgRef.current) return;
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    movedRef.current = true;
    const delta = { x: point.x - drag.start.x, y: point.y - drag.start.y };
    drag.nodeIds.forEach((id) => { const origin = drag.origins.get(id); if (origin) props.onNodeMove(id, origin.x + delta.x, origin.y + delta.y); });
  }

  function endMove(event: PointerEvent<SVGSVGElement>) {
    const selection = selectionRef.current;
    if (selection && selection.pointerId === event.pointerId && svgRef.current) {
      selectionRef.current = null;
      const end = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      const ids = selectionBoxIds(props.nodes, selection.start, end);
      props.onMultiSelect(selection.additive ? Array.from(new Set([...props.selectedNodeIds, ...ids])) : ids);
      if (ids.length === 1) props.onNodeClick(ids[0]!);
      setSelectionBox(null);
      return;
    }
    if (panRef.current?.pointerId === event.pointerId) {
      panRef.current = null;
      return;
    }
    const linkLabelDrag = linkLabelDragRef.current;
    if (linkLabelDrag?.pointerId === event.pointerId) {
      linkLabelDragRef.current = null;
      const key = `${linkLabelDrag.linkId}:${linkLabelDrag.nodeId}`;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        props.onLinkEndpointLabelMove(
          linkLabelDrag.linkId,
          linkLabelDrag.nodeId,
          linkLabelDrag.origin.x + point.x - linkLabelDrag.start.x,
          linkLabelDrag.origin.y + point.y - linkLabelDrag.start.y,
        );
      }
      setLinkLabelPositions((current) => { const next = { ...current }; delete next[key]; return next; });
      movedRef.current = false;
      return;
    }
    const labelDrag = labelDragRef.current;
    if (labelDrag?.pointerId === event.pointerId) {
      labelDragRef.current = null;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        props.onNodeLabelMove(labelDrag.nodeId, labelDrag.origin.x + point.x - labelDrag.start.x, labelDrag.origin.y + point.y - labelDrag.start.y);
      }
      setLabelPositions((current) => { const next = { ...current }; delete next[labelDrag.nodeId]; return next; });
      movedRef.current = false;
      return;
    }
    const drawingDrag = drawingDragRef.current;
      if (drawingDrag?.pointerId === event.pointerId) {
      drawingDragRef.current = null;
      if (movedRef.current && svgRef.current) {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
        const x = drawingDrag.origin.x + point.x - drawingDrag.start.x;
        const y = drawingDrag.origin.y + point.y - drawingDrag.start.y;
        setDrawingPositions((current) => { const next = { ...current }; delete next[drawingDrag.drawingId]; return next; });
        props.onDrawingMove(drawingDrag.drawingId, x, y);
      }
      movedRef.current = false;
      return;
    }
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (movedRef.current && svgRef.current) {
      const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
      const delta = { x: point.x - drag.start.x, y: point.y - drag.start.y };
      lastMoveRef.current = drag.origins;
      drag.nodeIds.forEach((id) => { const origin = drag.origins.get(id); if (origin) props.onNodeMoveEnd(id, origin.x + delta.x, origin.y + delta.y); });
    }
  }

  function dropTemplate(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDropActive(false);
    const templateId = event.dataTransfer.getData("application/x-network-simulator-template");
    if (!templateId || !svgRef.current) return;
    const point = scenePoint(event.clientX, event.clientY, svgRef.current.getBoundingClientRect());
    props.onDropTemplate(templateId, point.x, point.y);
  }

  return <div className={`sim-canvas-surface ${dropActive ? "is-drop-active" : ""}`} onDragOver={(event) => { if (event.dataTransfer.types.includes("application/x-network-simulator-template")) { event.preventDefault(); event.dataTransfer.dropEffect = "copy"; setDropActive(true); } }} onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as globalThis.Node | null)) setDropActive(false); }} onDrop={dropTemplate}>
    <svg ref={svgRef} viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} role="group" aria-label="Network topology canvas" tabIndex={0} onWheel={(event) => { if (event.ctrlKey || event.metaKey) { event.preventDefault(); props.onZoom(event.deltaY > 0 ? -0.1 : 0.1); } }} onPointerDown={startPan} onPointerMove={moveNode} onPointerUp={endMove} onPointerCancel={endMove} onKeyDown={onKeyDown} onKeyUp={onKeyUp} onBlur={onBlur}  onClick={(event) => {
      if (panMovedRef.current) { panMovedRef.current = false; return; }
      if (event.target === event.currentTarget || (event.target as Element).tagName === "rect") {
        const point = scenePoint(event.clientX, event.clientY, svgRef.current!.getBoundingClientRect());
        props.onCanvasClick(point);
      }
    }} onContextMenu={(event) => { if ((event.target as Element).tagName === "rect" && !(event.target as Element).closest(".sim-node, .sim-link, .sim-drawing")) props.onClearSelection(); }}>
      <defs><pattern id="sim-grid" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="currentColor" /></pattern></defs>
      <rect className={`sim-grid-fill ${props.gridVisible ? "" : "is-hidden"}`} x={view.x} y={view.y} width={view.width} height={view.height} fill="url(#sim-grid)" />
      {orderedDrawings.map(({ drawing, markup }) => {
        const position = drawingPositions[drawing.drawing_id] ?? { x: Number(drawing.x ?? 0), y: Number(drawing.y ?? 0) };
        const selected = props.selectedDrawingId === drawing.drawing_id;
        const width = Number(drawing.width ?? 160); const height = Number(drawing.height ?? 90);
        return <g key={drawing.drawing_id} className={`sim-drawing ${selected ? "is-selected" : ""}`} transform={`translate(${position.x},${position.y}) rotate(${Number(drawing.rotation ?? 0)})`} onPointerDown={(event) => startDrawingMove(event, drawing)} onClick={(event) => { event.stopPropagation(); props.onDrawingClick(drawing.drawing_id); }} onContextMenu={() => props.onDrawingClick(drawing.drawing_id)}>
          {selected && <rect className="sim-drawing-selection" x="-3" y="-3" width={width + 6} height={height + 6} />}
          <g dangerouslySetInnerHTML={{ __html: markup }} />
        </g>;
      })}
      {props.links.map((link) => {
        const endpoints = (link.nodes ?? []).map((endpoint) => ({ endpoint, node: nodeMap.get(endpoint.node_id) })).filter((item): item is { endpoint: LinkEndpoint; node: Node } => Boolean(item.node));
        const source = endpoints[0];
        if (!source || endpoints.length < 2) return null;
        const selected = props.selection?.kind === "link" && props.selection.id === link.link_id;
        const sourcePoint = portPoint(source.node, source.endpoint);
        return <g key={link.link_id} className={`sim-link ${selected ? "is-selected" : ""} ${link.suspend ? "is-suspended" : ""}`} onClick={(event) => { event.stopPropagation(); props.onLinkClick(link.link_id); }} onContextMenu={() => props.onLinkClick(link.link_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") props.onLinkClick(link.link_id); }} role="button" aria-label={`Ethernet segment connecting ${endpoints.map(({ node }) => node.name).join(", ")}`} tabIndex={0}>
          {endpoints.slice(1).map(({ endpoint, node }) => { const point = portPoint(node, endpoint); return <g key={`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`}><line className="sim-link-stroke" x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} style={{ stroke: selected ? "#1683df" : "#555", strokeWidth: selected ? 2.5 : 2, strokeDasharray: link.suspend ? "6 4" : undefined }} /><line className="sim-link-hit" x1={sourcePoint.x} y1={sourcePoint.y} x2={point.x} y2={point.y} /></g>; })}
          {endpoints.map(({ endpoint, node }, index) => {
            const other = endpoints[index === 0 ? 1 : 0]?.node;
            const dx = other ? Number(other.x ?? 0) - Number(node.x ?? 0) : 0;
            const dy = other ? Number(other.y ?? 0) - Number(node.y ?? 0) : 0;
            const distance = Math.hypot(dx, dy) || 1;
            return <circle key={`status:${endpoint.node_id}`} className={`sim-link-status ${node.status === "started" ? "running" : "stopped"}`} cx={Number(node.x ?? 0) + dx / distance * 27} cy={Number(node.y ?? 0) + dy / distance * 27} r="3" />;
          })}
          {endpoints.map(({ endpoint, node }) => {
            const localPort = localPortPoint(portIndex(node, endpoint), portCount(node));
            const key = `${link.link_id}:${endpoint.node_id}`;
            const savedLabel = endpoint.label;
            const text = savedLabel ? String(savedLabel.text ?? "") : portLabel(node, endpoint);
            if (!props.showInterfaceLabels || !text) return null;
            const position = linkLabelPositions[key] ?? { x: Number(savedLabel?.x ?? localPort.x + 8), y: Number(savedLabel?.y ?? localPort.y + 4) };
            return <g key={`label:${endpoint.node_id}`} transform={`translate(${Number(node.x ?? 0)},${Number(node.y ?? 0)})`}>
              <text className="sim-link-label" x={position.x} y={position.y} transform={Number(savedLabel?.rotation) ? `rotate(${Number(savedLabel?.rotation)}, ${position.x}, ${position.y})` : undefined} onPointerDown={(event) => startLinkLabelMove(event, link, endpoint, node)} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.stopPropagation()}>{text}</text>
            </g>;
          })}
        </g>;
      })}
      {props.connecting && props.linkEndpoints.length === 1 && cursorPoint && (() => {
        const endpoint = props.linkEndpoints[0]!;
        const node = nodeMap.get(endpoint.node_id);
        if (!node) return null;
        const point = portPoint(node, endpoint);
        return <line className="sim-link-preview" x1={point.x} y1={point.y} x2={cursorPoint.x} y2={cursorPoint.y} />;
      })()}
      {props.nodes.map((node) => {
        const x = Number(node.x ?? 0);
        const y = Number(node.y ?? 0);
        const selected = (props.selection?.kind === "node" && props.selection.id === node.node_id) || props.selectedNodeIds.includes(node.node_id);
        const symbol = resolveCanvasSymbol(node.node_type ?? "", node.symbol, props.symbolDefaults, props.customSymbolIds);
        const Icon = symbol.kind === "default" ? iconForSymbolName(symbol.name, node.node_type ?? "") : iconForType(node.node_type ?? "");
        const label = node.label && typeof node.label === "object" ? node.label as Record<string, unknown> : {};
        const labelPosition = labelPositions[node.node_id] ?? { x: Number(label.x ?? 0), y: Number(label.y ?? 36) };
        const suspended = node.status === "suspended" || node.status === "suspending";
        return <g key={node.node_id} transform={`translate(${x},${y})`} className={`sim-node ${selected ? "is-selected" : ""} ${props.connecting ? "is-connecting" : ""} ${node.locked || props.projectLocked ? "is-locked" : ""}`} onPointerDown={(event) => startMove(event, node)} onClick={(event) => { event.stopPropagation(); if (event.metaKey || event.ctrlKey) props.onMultiSelectNode(node.node_id); else if (!movedRef.current) props.onNodeClick(node.node_id); movedRef.current = false; }} onContextMenu={() => props.onNodeContext(node.node_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onNodeClick(node.node_id); } }} role="button" aria-label={`Node ${node.name}`} tabIndex={0}>
          {selected && <rect className="sim-node-selection" x="-33" y="-33" width="66" height="74" rx="6" />}
          {symbol.kind === "custom" ? <AuthenticatedSymbolImage symbolId={symbol.symbolId} /> : <Icon className="sim-node-icon" x={-24} y={-24} width={48} height={48} strokeWidth={1.5} />}
          <circle className={`sim-node-status ${node.status === "started" ? "running" : suspended ? "suspended" : "stopped"}`} cx="22" cy="-21" r="4" />
          {(node.locked || props.projectLocked) && <text className="sim-node-lock" x="-22" y="-17" aria-label="Locked">🔒</text>}
          <text className="sim-node-name" data-node-label="true" x={labelPosition.x} y={labelPosition.y} onPointerDown={(event) => startLabelMove(event, node)} onClick={(event) => event.stopPropagation()}>{node.name}</text>
        </g>;
        })}
      {selectionBox && <rect className="sim-selection-box" x={selectionBox.x} y={selectionBox.y} width={selectionBox.width} height={selectionBox.height} />}
      {props.nodes.length === 0 && <g className="sim-canvas-empty"><Network x={-16} y={-30} width={32} height={32} /><text y="54">Your canvas is ready</text><text className="sim-empty-subtitle" y="77">Drop a device here or add one from the library on the left.</text></g>}
    </svg>
  </div>;
}

export function AuthenticatedSymbolImage({ symbolId }: { symbolId: string }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    setUrl(null);
    void api.symbolRaw(symbolId).then((blob) => {
      objectUrl = URL.createObjectURL(blob);
      if (cancelled) URL.revokeObjectURL(objectUrl);
      else setUrl(objectUrl);
    }).catch(() => {});
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [symbolId]);
  return url
    ? <image className="sim-node-symbol" href={url} x="-23" y="-32" width="46" height="38" preserveAspectRatio="xMidYMid meet" />
    : null;
}

