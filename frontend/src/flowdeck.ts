import type { Node } from "./api";

export const DEFAULT_FLOWDECK_URL = "http://127.0.0.1:3095";
const FLOWDECK_URL_KEY = "network-simulator.flowdeck-url";

/** Docker nodes whose image name contains "openvswitch" (case-insensitive). */
export function isOpenVSwitchNode(node: Node): boolean {
  return node.node_type === "docker" && typeof node.image === "string" && node.image.toLowerCase().includes("openvswitch");
}

/** Flowdeck page for a node's container; trailing slashes on the base are trimmed. */
export function flowdeckUrlFor(node: Node, base: string): string {
  return `${base.replace(/\/+$/, "")}/?container=${encodeURIComponent(node.name)}`;
}

export function getFlowdeckBaseUrl(): string {
  try { return window.localStorage.getItem(FLOWDECK_URL_KEY)?.trim() || DEFAULT_FLOWDECK_URL; } catch { return DEFAULT_FLOWDECK_URL; }
}

export function saveFlowdeckBaseUrl(value: string) {
  try {
    const trimmed = value.trim();
    if (trimmed) window.localStorage.setItem(FLOWDECK_URL_KEY, trimmed);
    else window.localStorage.removeItem(FLOWDECK_URL_KEY);
  } catch { /* storage disabled */ }
}
