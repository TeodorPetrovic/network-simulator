import type { Node, ResourceMonitor, TopologySnapshot } from "./api";

export type HealthTone = "success" | "warning" | "danger" | "neutral";
export type HealthIssue = { id: string; title: string; detail: string; tone: HealthTone; nodeId?: string };
export type DeviceState = "running" | "stopped" | "suspended" | "unavailable" | "unknown";
export type ProjectHealth = { label: string; tone: HealthTone; summary: string; counts: Record<DeviceState, number>; issues: HealthIssue[] };

export function deviceState(node: Node): DeviceState {
  if (node.runtime_missing === true || node.runtime_supported === false || ["error", "failed", "crashed", "dead"].includes(node.status ?? "")) return "unavailable";
  if (node.status === "started") return "running";
  if (node.status === "stopped") return "stopped";
  if (node.status === "suspended") return "suspended";
  return "unknown";
}

export function projectHealth(topology: TopologySnapshot, connection = "connected", resources?: ResourceMonitor | null, resourceError?: string, now = Date.now()): ProjectHealth {
  const counts: ProjectHealth["counts"] = { running: 0, stopped: 0, suspended: 0, unavailable: 0, unknown: 0 };
  const issues: HealthIssue[] = [];
  for (const node of topology.nodes) {
    const state = deviceState(node);
    counts[state]++;
    if (state === "unavailable") issues.push({ id: node.node_id, title: node.name, nodeId: node.node_id, tone: "danger", detail: node.runtime_missing === true ? "Runtime data is missing. Check the device image or container before starting it." : node.runtime_supported === false ? "This runtime is unavailable. Check runtime support on the controller." : "The device reported a failure. Inspect its configuration and project log." });
    else if (state === "unknown") issues.push({ id: node.node_id, title: node.name, nodeId: node.node_id, tone: "warning", detail: `Device state is ${node.status || "unknown"}. Refresh the topology to check its current state.` });
    else if (state === "suspended") issues.push({ id: node.node_id, title: node.name, nodeId: node.node_id, tone: "warning", detail: "Device is suspended. Resume it when you want it to participate in the lab." });
    if (node.isolated === true) issues.push({ id: `${node.node_id}-isolated`, title: node.name, nodeId: node.node_id, tone: "warning", detail: "Device interfaces are isolated. Reconnect them from the device menu when ready." });
  }
  const nodeIds = new Set(topology.nodes.map((node) => node.node_id));
  const suspendedLinks = topology.links.filter((link) => link.suspend === true).length;
  const brokenLinks = topology.links.filter((link) => link.nodes.length !== 2 || link.nodes.some((endpoint) => !nodeIds.has(endpoint.node_id))).length;
  if (brokenLinks) issues.push({ id: "broken-links", title: `${brokenLinks} incomplete link${brokenLinks === 1 ? "" : "s"}`, tone: "danger", detail: "A link is missing a device endpoint. Inspect and reconnect it on the map." });
  if (suspendedLinks) issues.push({ id: "suspended-links", title: `${suspendedLinks} suspended link${suspendedLinks === 1 ? "" : "s"}`, tone: "warning", detail: "Traffic is paused on these links. Resume them from the link menu when ready." });
  if (resourceError) issues.push({ id: "metrics", title: "Host metrics unavailable", tone: "warning", detail: "Refresh Resources to check current CPU and memory usage." });
  else if (resources) {
    const fresh = resources.sampled_at_unix_ms != null && now - resources.sampled_at_unix_ms < 30_000 && now >= resources.sampled_at_unix_ms;
    if (!fresh) issues.push({ id: "metrics", title: "Host metrics are out of date", tone: "warning", detail: "Refresh Resources to get a current host sample." });
    else {
      const { cpu_usage_percent: cpu, memory_total_bytes: total, memory_used_bytes: used } = resources.host;
      if (cpu != null && cpu >= 90) issues.push({ id: "cpu", title: `High host CPU · ${Math.round(cpu)}%`, tone: "warning", detail: "The latest sample is at least 90%. Stop unused devices or check the resource monitor." });
      if (total != null && total > 0 && used != null && used / total >= 0.9) issues.push({ id: "memory", title: `High host memory · ${Math.round(used / total * 100)}%`, tone: "warning", detail: "The latest sample is at least 90%. Stop unused devices or reduce their memory allocation." });
    }
  }
  const result = (label: string, tone: HealthTone, summary: string): ProjectHealth => ({ label, tone, summary, counts, issues });
  if (connection !== "connected") return result(connection === "checking" ? "Checking" : connection === "reconnecting" ? "Reconnecting" : "Unavailable", connection === "disconnected" ? "danger" : "warning", "Live updates are unavailable. Device counts reflect the last known state; reconnect before relying on them.");
  if (issues.some((issue) => issue.tone === "danger")) return result("Needs attention", "danger", "Reported device or link problems need attention before this lab can run normally.");
  if (issues.length) return result("Needs attention", "warning", "Review the items below. Suspended devices and links may be intentional.");
  if (!topology.nodes.length) return result("No devices", "neutral", "Add devices and connect them to build this lab.");
  if (counts.stopped === topology.nodes.length) return result("Stopped", "neutral", "All devices are stopped. Start the devices you need to run this lab.");
  if (counts.stopped) return result("Partially running", "warning", `${counts.running} of ${topology.nodes.length} devices are running. Start the remaining devices if the full lab is needed.`);
  return result("Healthy", "success", "All devices report running, with no reported runtime or link problems.");
}
