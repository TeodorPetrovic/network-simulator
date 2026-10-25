import type { Drawing, LinkEndpoint, Node } from "../api";

export type DrawingTool = "rectangle" | "ellipse" | "line" | "text" | "image";

function interfaceCount(node: Node): number {
  const rawCount = Number(node.adapters ?? node.adapter_count ?? node.ports_count ?? (node.node_type === "vpcs" || node.node_type === "dynamips" ? 1 : 4));
  const minimum = node.node_type === "qemu" ? 0 : 1;
  return Number.isFinite(rawCount) ? Math.max(minimum, Math.floor(rawCount)) : 4;
}

export function interfacesForNode(node: Node): LinkEndpoint[] {
  const count = interfaceCount(node);
  return Array.from({ length: count }, (_, index) => node.node_type === "qemu"
    ? { node_id: node.node_id, adapter_number: index, port_number: 0 }
    : { node_id: node.node_id, adapter_number: 0, port_number: index });
}

export function availableInterfacesForNode(node: Node, links: Array<{ nodes?: LinkEndpoint[] }>): LinkEndpoint[] {
  const used = new Set(links.flatMap((link) => link.nodes ?? []).map((endpoint) => `${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`));
  return interfacesForNode(node).filter((endpoint) => !used.has(`${endpoint.node_id}:${endpoint.adapter_number}:${endpoint.port_number}`));
}

export function endpointName(node: Node, endpoint: LinkEndpoint): string {
  const port = node.node_type === "qemu" ? endpoint.adapter_number : endpoint.port_number;
  if (node.node_type === "qemu") return `Ethernet${port}`;
  if (node.node_type === "dynamips") return `Ethernet${port}`;
  return `eth${port}`;
}

export function defaultEndpointLabelPosition(node: Node, endpoint: LinkEndpoint): { x: number; y: number } {
  const count = interfaceCount(node);
  const index = node.node_type === "qemu" ? endpoint.adapter_number : endpoint.port_number;
  const columns = Math.min(8, count);
  const row = Math.floor(index / 8);
  const column = index % 8;
  const portsOnRow = Math.min(columns, count - row * 8);
  return {
    x: Math.round((column - (portsOnRow - 1) / 2) * 13 + 8),
    y: 27 + row * 10 + 4,
  };
}

export function drawingSvg(tool: DrawingTool, options: {
  width: number;
  height: number;
  stroke?: string;
  fill?: string;
  strokeWidth?: number;
  text?: string;
  imageDataUrl?: string;
}): string {
  const width = Math.max(1, Math.round(options.width));
  const height = Math.max(1, Math.round(options.height));
  const stroke = escapeXml(options.stroke ?? "#2563eb");
  const fill = escapeXml(options.fill ?? "none");
  const strokeWidth = Math.max(1, Number(options.strokeWidth ?? 2));
  const start = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`;
  let shape: string;
  switch (tool) {
    case "rectangle": shape = `<rect x="1" y="1" width="${Math.max(0, width - 2)}" height="${Math.max(0, height - 2)}" fill="${fill}" stroke="${stroke}" stroke-width="${strokeWidth}"/>`; break;
    case "ellipse": shape = `<ellipse cx="${width / 2}" cy="${height / 2}" rx="${Math.max(0, (width - strokeWidth) / 2)}" ry="${Math.max(0, (height - strokeWidth) / 2)}" fill="${fill}" stroke="${stroke}" stroke-width="${strokeWidth}"/>`; break;
    case "line": shape = `<line x1="1" y1="1" x2="${Math.max(1, width - 1)}" y2="${Math.max(1, height - 1)}" stroke="${stroke}" stroke-width="${strokeWidth}"/>`; break;
    case "text": shape = `<text x="2" y="${Math.min(height - 2, 20)}" fill="${stroke}" font-size="16">${escapeXml(options.text ?? "Text")}</text>`; break;
    case "image": {
      const dataUrl = options.imageDataUrl ?? "";
      if (!/^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/=]+$/i.test(dataUrl)) throw new Error("Choose a PNG, JPEG, GIF, or WebP image.");
      shape = `<image href="${dataUrl}" x="0" y="0" width="${width}" height="${height}" preserveAspectRatio="xMidYMid meet"/>`;
      break;
    }
  }
  return `${start}${shape}</svg>`;
}

export function reorderDrawings<T extends Pick<Drawing, "drawing_id" | "z_index">>(drawings: T[], drawingId: string, direction: "front" | "back"): T[] {
  const ordered = [...drawings].sort((left, right) => Number(left.z_index ?? 0) - Number(right.z_index ?? 0));
  const currentIndex = ordered.findIndex((drawing) => drawing.drawing_id === drawingId);
  if (currentIndex < 0) return drawings;
  const [picked] = ordered.splice(currentIndex, 1);
  if (direction === "front") ordered.push(picked);
  else ordered.unshift(picked);
  return ordered.map((drawing, index) => ({ ...drawing, z_index: index }));
}

export function selectionBoxIds(nodes: Node[], first: { x: number; y: number }, second: { x: number; y: number }): string[] {
  const minX = Math.min(first.x, second.x);
  const maxX = Math.max(first.x, second.x);
  const minY = Math.min(first.y, second.y);
  const maxY = Math.max(first.y, second.y);
  return nodes.filter((node) => {
    const x = Number(node.x ?? 0);
    const y = Number(node.y ?? 0);
    return x >= minX - 56 && x <= maxX + 56 && y >= minY - 36 && y <= maxY + 36;
  }).map((node) => node.node_id);
}

export function serializeSvgScreenshot(source: SVGSVGElement): string {
  const clone = source.cloneNode(true) as SVGSVGElement;
  clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  const originals = [source, ...Array.from(source.querySelectorAll("*"))];
  const copies = [clone, ...Array.from(clone.querySelectorAll("*"))];
  for (let index = 0; index < copies.length; index += 1) {
    const element = copies[index];
    const original = originals[index];
    if (!element || !original) continue;
    const computed = getComputedStyle(original);
    for (const property of ["fill", "stroke", "stroke-width", "font-family", "font-size", "font-weight", "opacity", "filter"]) {
      const value = computed.getPropertyValue(property);
      if (value && value !== "none") element.setAttribute(property, value);
    }
  }
  return new XMLSerializer().serializeToString(clone);
}

export async function svgScreenshotPng(svgText: string, width: number, height: number): Promise<Blob> {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("PNG export is not available in this browser.");
  const imageUrl = URL.createObjectURL(new Blob([svgText], { type: "image/svg+xml;charset=utf-8" }));
  try {
    const image = new Image();
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("The map image could not be rendered as PNG."));
      image.src = imageUrl;
    });
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error("PNG export could not be created.")), "image/png");
    });
  } finally {
    URL.revokeObjectURL(imageUrl);
  }
}

function escapeXml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" })[character] ?? character);
}
