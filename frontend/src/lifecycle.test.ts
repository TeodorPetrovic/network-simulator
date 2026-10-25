import { describe, expect, it } from "vitest";
import type { Node } from "./api";
import { canApplyLifecycle, nodesForLifecycle } from "./lifecycle";

function node(node_type: string, status: string): Node {
  return { node_id: `${node_type}-${status}`, name: node_type, node_type, status };
}

describe("node lifecycle actions", () => {
  it("filters always-running built-ins and cloud/NAT actions", () => {
    for (const type of ["ethernet_switch", "ethernet_hub", "cloud", "nat"]) {
      const item = node(type, "started");
      expect(canApplyLifecycle(item, "stop")).toBe(false);
      expect(canApplyLifecycle(item, "suspend")).toBe(false);
      expect(canApplyLifecycle(item, "reload")).toBe(false);
    }
    expect(nodesForLifecycle([node("cloud", "started"), node("docker", "stopped")], "start")).toEqual([
      node("docker", "stopped"),
    ]);
  });

  it("omits VPCS suspend and actions for nodes in the wrong state", () => {
    expect(canApplyLifecycle(node("vpcs", "started"), "suspend")).toBe(false);
    expect(canApplyLifecycle(node("vpcs", "started"), "reload")).toBe(true);
    expect(canApplyLifecycle(node("qemu", "stopped"), "stop")).toBe(false);
    expect(canApplyLifecycle(node("qemu", "suspended"), "stop")).toBe(true);
    expect(canApplyLifecycle(node("qemu", "suspended"), "start")).toBe(true);
  });

  it("offers Dynamips controls according to the returned runtime status", () => {
    const stopped = node("dynamips", "stopped");
    expect(canApplyLifecycle(stopped, "start")).toBe(true);
    expect(canApplyLifecycle(stopped, "stop")).toBe(false);
    for (const status of ["started", "suspended"]) {
      const item = node("dynamips", status);
      expect(canApplyLifecycle(item, "start")).toBe(status === "suspended");
      expect(canApplyLifecycle(item, "stop")).toBe(true);
      expect(canApplyLifecycle(item, "suspend")).toBe(status === "started");
      expect(canApplyLifecycle(item, "reload")).toBe(status === "started");
    }
  });
});
