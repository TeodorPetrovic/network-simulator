import type { LinkEndpoint, Node, RuntimeTemplate, TopologySnapshot } from "./api";

export function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Something went wrong. Please try again.";
}

export function samePort(left: LinkEndpoint, right: LinkEndpoint) {
  return left.node_id === right.node_id && left.adapter_number === right.adapter_number && left.port_number === right.port_number;
}

export function generatedNodeName(template: RuntimeTemplate, nodes: Node[], pendingNames: ReadonlySet<string>) {
  let base: string;
  if (template.template_type === "vpcs") base = "VPCS";
  else if (template.template_type === "dynamips") base = "Cisco Router";
  else if (template.template_type === "qemu") {
    const name = template.name.replace(/\.(?:qcow2?|raw|img|vmdk|vdi)$/i, "").replace(/\s+\d+(?:\.\d+)*.*$/, "").trim();
    base = name || "QEMU VM";
  } else if (template.template_type === "docker") {
    const imageName = ("image" in template && typeof template.image === "string" ? template.image : "").split("/").at(-1)?.split(":")[0] || template.name;
    base = imageName.charAt(0).toUpperCase() + imageName.slice(1);
  } else if (template.template_type === "ethernet_switch") base = "Switch";
  else if (template.template_type === "ethernet_hub") base = "Hub";
  else if (template.template_type === "cloud") base = "Cloud";
  else if (template.template_type === "nat") base = "NAT";
  else base = "VPCS";
  const usedNames = new Set(nodes.map((node) => node.name));
  let next = 1;
  while (usedNames.has(`${base} ${next}`) || pendingNames.has(`${base} ${next}`)) next += 1;
  return `${base} ${next}`;
}

export function upsertNode(nodes: Node[], node: Node) {
  return nodes.some((item) => item.node_id === node.node_id)
    ? nodes.map((item) => item.node_id === node.node_id ? node : item)
    : [...nodes, node];
}

export function upsertLink(links: TopologySnapshot["links"], link: TopologySnapshot["links"][number]) {
  return links.some((item) => item.link_id === link.link_id)
    ? links.map((item) => item.link_id === link.link_id ? link : item)
    : [...links, link];
}

