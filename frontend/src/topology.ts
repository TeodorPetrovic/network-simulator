import type { ProjectEvent, TopologySnapshot } from "./api";

const idFor = (event: ProjectEvent): string | undefined => {
  const value = event.event;
  const id = value.node_id ?? value.link_id ?? value.drawing_id;
  return typeof id === "string" ? id : undefined;
};

function replaceById<T extends Record<string, unknown>>(
  items: T[],
  idField: string,
  id: string,
  value: Record<string, unknown>,
): T[] {
  const index = items.findIndex((item) => item[idField] === id);
  if (index === -1) return [...items, value as T];
  return items.map((item, itemIndex) => (itemIndex === index ? { ...item, ...value } : item));
}

function removeById<T extends Record<string, unknown>>(
  items: T[],
  idField: string,
  id: string,
): T[] {
  return items.filter((item) => item[idField] !== id);
}

export function applyProjectEvent(
  snapshot: TopologySnapshot,
  message: ProjectEvent,
): TopologySnapshot {
  const id = idFor(message);
  if (!id) return snapshot;

  const [kind, action] = message.action.split(".");
  const collection = kind === "node" ? "nodes" : kind === "link" ? "links" : kind === "drawing" ? "drawings" : null;
  if (!collection || !["created", "updated", "deleted"].includes(action)) return snapshot;

  const items = snapshot[collection] as Array<Record<string, unknown>>;
  const idField = `${kind}_id`;
  const updated = action === "deleted"
    ? removeById(items, idField, id)
    : replaceById(items, idField, id, message.event);
  return {
    ...snapshot,
    [collection]: updated,
    event_cursor: Math.max(snapshot.event_cursor, message.sequence),
  };
}

const svgNamespace = "http://www.w3.org/2000/svg";
const drawingElements = new Set([
  "svg", "g", "rect", "circle", "ellipse", "line", "polyline", "polygon", "path", "image",
  "text", "tspan", "defs", "lineargradient", "radialgradient", "stop", "clippath",
]);
const drawingAttributes = new Set([
  "xmlns", "width", "height", "viewbox", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy",
  "r", "rx", "ry", "d", "points", "fill", "fill-opacity", "stroke", "stroke-opacity",
  "stroke-width", "stroke-linecap", "stroke-linejoin", "opacity", "transform", "font-family",
  "font-size", "font-weight", "text-anchor", "dominant-baseline", "letter-spacing", "id",
  "gradientunits", "gradienttransform", "offset", "stop-color", "stop-opacity", "clip-path",
  "href", "preserveaspectratio",
]);

export function sanitizeDrawingSvg(source: string): string {
  const parsed = new DOMParser().parseFromString(source, "image/svg+xml");
  const root = parsed.documentElement;
  if (root.localName.toLowerCase() !== "svg" || parsed.querySelector("parsererror")) return "";

  const clean = (element: Element): Element | null => {
    const tag = element.localName.toLowerCase();
    if (!drawingElements.has(tag)) return null;
    const copy = document.createElementNS(svgNamespace, tag);
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim();
      if (!drawingAttributes.has(name) || name.startsWith("on")) continue;
      if (name === "xmlns" && value !== svgNamespace) continue;
      if (name === "id" && !/^[-\w:.]+$/.test(value)) continue;
      if (name === "transform" && !/^[\w.,\s()+-]+$/.test(value)) continue;
      if (name === "d" && !/^[\w.,\s+-]+$/.test(value)) continue;
      if (name === "href" && !/^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/=]+$/i.test(value)) continue;
      if (name === "fill" || name === "clip-path") {
        if (/url\(/i.test(value) && !/^url\(#[-\w:.]+\)$/.test(value)) continue;
      }
      copy.setAttribute(name === "viewbox" ? "viewBox" : attribute.name, value);
    }
    for (const child of Array.from(element.childNodes)) {
      if (child.nodeType === Node.TEXT_NODE) {
        copy.appendChild(document.createTextNode(child.textContent ?? ""));
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        const sanitized = clean(child as Element);
        if (sanitized) copy.appendChild(sanitized);
      }
    }
    return copy;
  };

  const sanitizedRoot = clean(root);
  return sanitizedRoot ? new XMLSerializer().serializeToString(sanitizedRoot) : "";
}
