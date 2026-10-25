import type { Node } from "./api";

export type LifecycleAction = "start" | "stop" | "suspend" | "reload";

const runtimeNodeTypes = new Set(["docker", "qemu", "vpcs", "dynamips"]);

export function canApplyLifecycle(node: Node, action: LifecycleAction): boolean {
  const nodeType = node.node_type ?? "";
  const status = node.status ?? "stopped";

  if ((nodeType === "cloud" || nodeType === "nat") && action !== "start") return false;
  if (nodeType === "vpcs" && action === "suspend") return false;
  if (!runtimeNodeTypes.has(nodeType) && nodeType !== "cloud" && nodeType !== "nat") return false;

  switch (action) {
    case "start":
      return status !== "started";
    case "stop":
      return status === "started" || status === "suspended";
    case "suspend":
    case "reload":
      return status === "started";
  }
}

export function nodesForLifecycle(nodes: Node[], action: LifecycleAction): Node[] {
  return nodes.filter((node) => canApplyLifecycle(node, action));
}
