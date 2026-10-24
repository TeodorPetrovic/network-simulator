import type { Node } from "./api";

export function pngScreenshotAvailable() {
  const canvas = document.createElement("canvas");
  return typeof canvas.toBlob === "function" && typeof canvas.toDataURL === "function";
}

export function spreadOverlappingNodes(nodes: Node[]) {
  const positions = new Map<string, number>();
  for (const node of nodes) {
    const key = `${Number(node.x ?? 0)}:${Number(node.y ?? 0)}`;
    positions.set(key, (positions.get(key) ?? 0) + 1);
  }
  const columns = Math.min(3, Math.max(1, Math.ceil(Math.sqrt(nodes.length))));
  return nodes.map((node, index) => {
    const x = Number(node.x ?? 0);
    const y = Number(node.y ?? 0);
    if ((positions.get(`${x}:${y}`) ?? 0) < 2) return node;
    const column = index % columns;
    const row = Math.floor(index / columns);
    return { ...node, x: x + (column - (columns - 1) / 2) * 150, y: y + row * 120 };
  });
}

