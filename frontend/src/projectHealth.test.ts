import { describe, expect, it } from "vitest";
import type { Node, ResourceMonitor, TopologySnapshot } from "./api";
import { projectHealth } from "./projectHealth";

const snapshot = (nodes: Partial<Node>[] = []): TopologySnapshot => ({ project: { project_id: "p", name: "Lab" }, nodes: nodes.map((node, i) => ({ node_id: `n${i}`, name: `Router ${i}`, node_type: "qemu", ...node })), links: [], drawings: [], event_cursor: 0 });

describe("project health", () => {
  it.each([
    [[], "No devices", "neutral"],
    [[{ status: "stopped" }], "Stopped", "neutral"],
    [[{ status: "started" }], "Healthy", "success"],
    [[{ status: "started" }, { status: "stopped" }], "Partially running", "warning"],
    [[{ status: "suspended" }], "Needs attention", "warning"],
    [[{}], "Needs attention", "warning"],
    [[{ status: "failed" }], "Needs attention", "danger"],
    [[{ status: "started", runtime_missing: true }], "Needs attention", "danger"],
    [[{ status: "stopped", runtime_supported: false }], "Needs attention", "danger"],
  ] as const)("assesses %j without assuming stopped or missing devices are healthy", (nodes, label, tone) => {
    expect(projectHealth(snapshot([...nodes]))).toMatchObject({ label, tone });
  });

  it("prioritizes the disconnected controller over last known running devices", () => {
    const health = projectHealth(snapshot([{ status: "started" }]), "reconnecting");
    expect(health.label).toBe("Reconnecting");
    expect(health.tone).not.toBe("success");
    expect(health.summary).toContain("last known state");
    expect(health.counts.running).toBe(1);
  });

  it("reports isolated devices, suspended links, and missing endpoints with next steps", () => {
    const topology = snapshot([{ status: "started", isolated: true }, { status: "started" }]);
    topology.links = [{ link_id: "l", suspend: true, nodes: [{ node_id: "n0", adapter_number: 0, port_number: 0 }, { node_id: "missing", adapter_number: 0, port_number: 0 }] }];
    expect(projectHealth(topology)).toMatchObject({ tone: "danger", issues: expect.arrayContaining([expect.objectContaining({ id: "n0-isolated", nodeId: "n0" }), expect.objectContaining({ id: "suspended-links" }), expect.objectContaining({ id: "broken-links" })]) });
  });

  it("uses fresh host measurements and reports old or failed measurements separately", () => {
    const resources: ResourceMonitor = { sampled_at_unix_ms: 100_000, host: { cpu_usage_percent: 94, cpu_count: 4, memory_used_bytes: 95, memory_total_bytes: 100, load_average: null }, controller: { memory_rss_bytes: null, thread_count: null } };
    const topology = snapshot([{ status: "started" }]);
    expect(projectHealth(topology, "connected", resources, "", 110_000).issues.map((issue) => issue.id)).toEqual(["cpu", "memory"]);
    expect(projectHealth(topology, "connected", resources, "", 150_000).issues).toEqual([expect.objectContaining({ id: "metrics", title: "Host metrics are out of date" })]);
    expect(projectHealth(topology, "connected", resources, "Request failed", 110_000).issues).toEqual([expect.objectContaining({ id: "metrics", title: "Host metrics unavailable" })]);
  });
});
