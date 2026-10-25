import type { ComponentType } from "react";
import { Cloud, Cpu, HardDrive, Monitor, Network, Router, Server } from "lucide-react";
import type { Drawing, LinkEndpoint, Node, RuntimeTemplate } from "../api";

type DrawingValues = { width: number; height: number; stroke: string; fill: string; strokeWidth: number; text: string; imageDataUrl?: string };

export function sameEndpoint(left: LinkEndpoint, right: LinkEndpoint) {
  return left.node_id === right.node_id && left.adapter_number === right.adapter_number && left.port_number === right.port_number;
}

export function portCount(node: Node): number {
  const value = Number(node.adapters ?? node.adapter_count ?? node.ports_count ?? (node.node_type === "vpcs" || node.node_type === "dynamips" ? 1 : 4));
  return Number.isFinite(value) ? Math.max(1, Math.min(16, value)) : 4;
}

export function nodePorts(node: Node): LinkEndpoint[] {
  return Array.from({ length: portCount(node) }, (_, index) => node.node_type === "qemu"
    ? { node_id: node.node_id, adapter_number: index, port_number: 0 }
    : { node_id: node.node_id, adapter_number: 0, port_number: index });
}

export function portIndex(node: Node, endpoint: LinkEndpoint) { return node.node_type === "qemu" ? endpoint.adapter_number : endpoint.port_number; }

export function localPortPoint(index: number, total: number) {
  const columns = Math.min(8, total);
  const row = Math.floor(index / 8);
  const column = index % 8;
  return { x: (column - (Math.min(columns, total - row * 8) - 1) / 2) * 13, y: 27 + row * 10 };
}

export function portPoint(node: Node, endpoint: LinkEndpoint) {
  void endpoint;
  return { x: Number(node.x ?? 0), y: Number(node.y ?? 0) };
}

export function portLabel(node: Node | undefined, endpoint: LinkEndpoint) {
  if (node?.node_type === "qemu") return `Ethernet${endpoint.adapter_number}`;
  if (node?.node_type === "dynamips") return `Ethernet${endpoint.port_number}`;
  if (["ethernet_switch", "ethernet_hub", "cloud", "nat"].includes(node?.node_type ?? "")) {
    const ports = node?.ports_mapping;
    const port = Array.isArray(ports) ? ports[endpoint.port_number] as { name?: unknown } | undefined : undefined;
    return typeof port?.name === "string" ? port.name : `Ethernet${endpoint.port_number}`;
  }
  return `eth${endpoint.port_number}`;
}

export function endpointLabel(node: Node, endpoint: LinkEndpoint) {
  const index = portIndex(node, endpoint);
  return node.node_type === "qemu" ? `e${index}` : node.node_type === "dynamips" ? `e${index}` : `${index}`;
}

export function iconForType(type: string): ComponentType<{ size?: number; className?: string; x?: number; y?: number; width?: number; height?: number; strokeWidth?: number }> {
  if (type === "qemu") return Monitor;
  if (type === "dynamips") return Router;
  if (type === "docker") return Server;
  if (type === "vpcs") return Cpu;
  if (type === "ethernet_switch" || type === "ethernet_hub" || type === "nat") return Network;
  if (type === "cloud") return Cloud;
  return HardDrive;
}

export function iconForSymbolName(name: string, type: string) {
  const key = name.toLocaleLowerCase();
  if (key.includes("router")) return Router;
  if (key.includes("switch") || key.includes("hub")) return Network;
  if (key.includes("cloud")) return Cloud;
  if (key === "pc" || key.includes("computer")) return Monitor;
  return iconForType(type);
}

export function kindLabel(kind: string) {
  return ({ dynamips: "Cisco IOS", qemu: "QEMU / KVM", docker: "Docker", vpcs: "VPCS", ethernet_switch: "Ethernet switch", ethernet_hub: "Ethernet hub", cloud: "Cloud", nat: "NAT", device: "Device" } as Record<string, string>)[kind] ?? kind;
}

export function deviceGroupLabel(kind: string) { return kind === "dynamips" ? "Dynamips · Cisco IOS" : kindLabel(kind); }

export function templateDescription(template: RuntimeTemplate) {
  if (template.template_type === "qemu") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · ${template.memory} MB RAM`;
  if (template.template_type === "docker") return `${template.adapters} NIC${template.adapters === 1 ? "" : "s"} · Docker image`;
  if (template.template_type === "dynamips") return `${template.platform} · Cisco IOS`;
  if (template.template_type === "ethernet_switch") return `${template.ports_mapping.length} Ethernet ports · VLAN switching`;
  if (template.template_type === "ethernet_hub") return `${template.ports_mapping.length} Ethernet ports · shared medium`;
  if (template.template_type === "cloud") return `${template.ports_mapping.length} host ports · Ethernet, TAP, or UDP`;
  if (template.template_type === "nat") return "Host NAT network · libvirt virbr0";
  return "Lightweight virtual PC";
}

export function imageName(node: Node) {
  const image = typeof node.image === "string" ? node.image : typeof node.hda_disk_image === "string" ? node.hda_disk_image : "";
  return image.split("/").at(-1) || kindLabel(node.node_type ?? "device");
}

export function valuesFromDrawing(drawing: Drawing): DrawingValues {
  const source = typeof drawing.svg === "string" ? drawing.svg : "";
  const parsed = new DOMParser().parseFromString(source, "image/svg+xml");
  const svg = parsed.documentElement;
  const shape = svg.firstElementChild;
  const getNumber = (name: string, fallback: number) => Number(svg.getAttribute(name) ?? drawing[name] ?? fallback) || fallback;
  const fill = shape?.getAttribute("fill") ?? "#dbeafe";
  return {
    width: getNumber("width", 180), height: getNumber("height", 100),
    stroke: shape?.getAttribute("stroke") ?? "#2563eb",
    fill: fill === "none" ? "#ffffff" : fill,
    strokeWidth: Number(shape?.getAttribute("stroke-width") ?? 2) || 2,
    text: shape?.textContent ?? "Text",
    ...(shape?.localName === "image" && shape.getAttribute("href") ? { imageDataUrl: shape.getAttribute("href")! } : {}),
  };
}

export function safeFileName(value: string) { return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "network-map"; }
export function messageFor(reason: unknown) { return reason instanceof Error ? reason.message : "The controller did not complete this request."; }

export function nodeHasAuxConsole(node: Node) {
  const properties = node.properties && typeof node.properties === "object" ? node.properties as Record<string, unknown> : {};
  return String(node.aux_type ?? properties.aux_type ?? "none") !== "none" && Number(properties.aux ?? node.aux_console_port ?? 0) > 0;
}
