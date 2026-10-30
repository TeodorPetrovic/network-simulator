import type { Node } from "../api";

export type NodePosition = { id: string; x: number; y: number };
const position = (node: Node): NodePosition => ({ id: node.node_id, x: Number(node.x ?? 0), y: Number(node.y ?? 0) });

export function alignNodes(nodes: Node[], mode: "left" | "right" | "top" | "bottom" | "horizontal" | "vertical"): NodePosition[] {
  if (nodes.length < 2) return nodes.map(position);
  const xs = nodes.map((node) => Number(node.x ?? 0));
  const ys = nodes.map((node) => Number(node.y ?? 0));
  const x = mode === "left" ? Math.min(...xs) : mode === "right" ? Math.max(...xs) : (Math.min(...xs) + Math.max(...xs)) / 2;
  const y = mode === "top" ? Math.min(...ys) : mode === "bottom" ? Math.max(...ys) : (Math.min(...ys) + Math.max(...ys)) / 2;
  return nodes.map((node) => ({ id: node.node_id, x: mode === "left" || mode === "right" || mode === "vertical" ? x : Number(node.x ?? 0), y: mode === "top" || mode === "bottom" || mode === "horizontal" ? y : Number(node.y ?? 0) }));
}

export function distributeNodes(nodes: Node[], axis: "horizontal" | "vertical"): NodePosition[] {
  if (nodes.length < 3) return nodes.map(position);
  const sorted = [...nodes].sort((a, b) => Number(a[axis === "horizontal" ? "x" : "y"] ?? 0) - Number(b[axis === "horizontal" ? "x" : "y"] ?? 0));
  const key = axis === "horizontal" ? "x" : "y";
  const start = Number(sorted[0]![key] ?? 0), end = Number(sorted.at(-1)![key] ?? 0), step = (end - start) / (sorted.length - 1);
  return sorted.map((node, index) => ({ id: node.node_id, x: key === "x" ? start + step * index : Number(node.x ?? 0), y: key === "y" ? start + step * index : Number(node.y ?? 0) }));
}
