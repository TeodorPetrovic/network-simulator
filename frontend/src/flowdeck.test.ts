import { afterEach, describe, expect, it } from "vitest";
import type { Node } from "./api";
import { DEFAULT_FLOWDECK_URL, flowdeckUrlFor, getFlowdeckBaseUrl, isOpenVSwitchNode, saveFlowdeckBaseUrl } from "./flowdeck";

afterEach(() => window.localStorage.clear());

const ovs: Node = { node_id: "s1", name: "SW 1/a", node_type: "docker", image: "Local/OpenVSwitch:3.3" };

describe("isOpenVSwitchNode", () => {
  it("matches docker nodes whose image contains openvswitch, case-insensitively", () => {
    expect(isOpenVSwitchNode(ovs)).toBe(true);
    expect(isOpenVSwitchNode({ ...ovs, image: "openvswitch/ovs:latest" })).toBe(true);
  });

  it("excludes non-docker nodes even when they carry an openvswitch image field", () => {
    expect(isOpenVSwitchNode({ node_id: "v", name: "VPCS1", node_type: "vpcs", image: "openvswitch" })).toBe(false);
  });

  it("excludes docker nodes with other images", () => {
    expect(isOpenVSwitchNode({ ...ovs, image: "alpine:3.20" })).toBe(false);
    expect(isOpenVSwitchNode({ node_id: "d", name: "D1", node_type: "docker" })).toBe(false);
  });
});

describe("flowdeckUrlFor", () => {
  it("builds the container URL with the node name encoded", () => {
    expect(flowdeckUrlFor(ovs, "http://127.0.0.1:3095")).toBe("http://127.0.0.1:3095/?container=SW%201%2Fa");
  });

  it("trims trailing slashes from the base", () => {
    expect(flowdeckUrlFor(ovs, "http://flow.local:3095///")).toBe("http://flow.local:3095/?container=SW%201%2Fa");
  });
});

describe("Flowdeck URL preference", () => {
  it("defaults to the local Flowdeck address and stores a trimmed override", () => {
    expect(getFlowdeckBaseUrl()).toBe(DEFAULT_FLOWDECK_URL);
    saveFlowdeckBaseUrl("  http://ctl:9000  ");
    expect(getFlowdeckBaseUrl()).toBe("http://ctl:9000");
    saveFlowdeckBaseUrl("   ");
    expect(getFlowdeckBaseUrl()).toBe(DEFAULT_FLOWDECK_URL);
  });
});
