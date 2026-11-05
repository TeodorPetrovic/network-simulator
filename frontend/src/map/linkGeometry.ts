import type { CSSProperties } from "react";

export const gns3LabelStyle = { fontFamily: "TypeWriter, monospace", fontSize: 10, fontWeight: 700 } as const;
export function svgTextStyle(value: unknown): CSSProperties {
  if (typeof value !== "string") return gns3LabelStyle;
  const result: CSSProperties = { ...gns3LabelStyle };
  for (const declaration of value.split(";")) {
    const separator = declaration.indexOf(":");
    if (separator < 1) continue;
    const key = declaration.slice(0, separator).trim().toLowerCase();
    const content = declaration.slice(separator + 1).trim();
    if (!content) continue;
    if (key === "fill" || key === "color") result.fill = content;
    else if (key === "font-family") result.fontFamily = content;
    else if (key === "font-size") result.fontSize = content;
    else if (key === "font-weight") result.fontWeight = content;
    else if (key === "font-style") result.fontStyle = content;
    else if (key === "opacity") result.opacity = Number(content);
  }
  return result;
}

export function linkDash(type: unknown) {
  return ["", undefined, "8 4", "1 4", "8 4 1 4", "8 4 1 4 1 4"][Number(type)] ?? undefined;
}

export function linkPath(from: { x: number; y: number }, to: { x: number; y: number }, style: Record<string, unknown>) {
  const kind = style.link_type;
  if (kind === "flowchart") {
    const middle = (from.x + to.x) / 2;
    return `M ${from.x} ${from.y} H ${middle} V ${to.y} H ${to.x}`;
  }
  if (kind === "bezier" || kind === "statemachine" || kind === "freeform") {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const length = Math.hypot(dx, dy) || 1;
    const bend = Number(style.bezier_curviness ?? 30);
    const offset = Array.isArray(style.control_offset) ? style.control_offset : null;
    const cx = offset ? Number(offset[0]) : (from.x + to.x) / 2 - dy / length * bend;
    const cy = offset ? Number(offset[1]) : (from.y + to.y) / 2 + dx / length * bend;
    return `M ${from.x} ${from.y} Q ${cx} ${cy} ${to.x} ${to.y}`;
  }
  return `M ${from.x} ${from.y} L ${to.x} ${to.y}`;
}

