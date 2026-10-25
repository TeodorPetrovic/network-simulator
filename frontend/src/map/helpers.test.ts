import { describe, expect, it } from "vitest";
import type { Node } from "../api";
import { availableInterfacesForNode, drawingSvg, endpointName, interfacesForNode, reorderDrawings, selectionBoxIds, serializeSvgScreenshot } from "./helpers";

describe("project map helpers", () => {
  it("filters occupied ports for the link context menu", () => {
    const node: Node = { node_id: "q1", name: "QEMU", node_type: "qemu", adapters: 2 };
    expect(availableInterfacesForNode(node, [{ nodes: [{ node_id: "q1", adapter_number: 0, port_number: 0 }] }])).toEqual([{ node_id: "q1", adapter_number: 1, port_number: 0 }]);
  });

  it("creates runtime-shaped interface choices for the picker", () => {
    const qemu: Node = { node_id: "q1", name: "QEMU", node_type: "qemu", adapters: 3 };
    const ports = interfacesForNode(qemu);
    expect(ports).toHaveLength(3);
    expect(ports[2]).toMatchObject({ adapter_number: 2, port_number: 0 });
    expect(endpointName(qemu, ports[2]!)).toBe("Ethernet2");
  });

  it("keeps the full QEMU and Dynamips adapter ranges available", () => {
    const qemuPorts = interfacesForNode({ node_id: "q1", name: "QEMU", node_type: "qemu", adapters: 275 });
    const dynamipsPorts = interfacesForNode({ node_id: "d1", name: "Dynamips", node_type: "dynamips", adapters: 99 });

    expect(qemuPorts).toHaveLength(275);
    expect(qemuPorts[274]).toMatchObject({ adapter_number: 274, port_number: 0 });
    expect(dynamipsPorts).toHaveLength(99);
    expect(dynamipsPorts[98]).toMatchObject({ adapter_number: 0, port_number: 98 });
    expect(interfacesForNode({ node_id: "q0", name: "Empty QEMU", node_type: "qemu", adapters: 0 })).toHaveLength(0);
  });

  it.each(["rectangle", "ellipse", "line", "text"] as const)("builds safe SVG for %s drawings", (tool) => {
    const svg = drawingSvg(tool, { width: 120, height: 60, text: "a < b & c" });
    expect(svg).toContain("<svg");
    expect(svg).toContain(tool === "rectangle" ? "<rect" : tool === "ellipse" ? "<ellipse" : tool === "line" ? "<line" : "<text");
    if (tool === "text") expect(svg).toContain("a &lt; b &amp; c");
  });

  it("accepts only embedded raster data for image drawings", () => {
    const svg = drawingSvg("image", { width: 40, height: 30, imageDataUrl: "data:image/png;base64,AA==" });
    expect(svg).toContain("<image href=\"data:image/png;base64,AA==\"");
    expect(() => drawingSvg("image", { width: 40, height: 30, imageDataUrl: "https://example.invalid/image.svg" })).toThrow("Choose a PNG");
  });

  it("moves a drawing to the requested z-order and renumbers the stack", () => {
    const drawings = [{ drawing_id: "a", z_index: 0 }, { drawing_id: "b", z_index: 1 }, { drawing_id: "c", z_index: 2 }];
    expect(reorderDrawings(drawings, "a", "front").map((drawing) => drawing.drawing_id)).toEqual(["b", "c", "a"]);
    expect(reorderDrawings(drawings, "c", "back").map((drawing) => drawing.drawing_id)).toEqual(["c", "a", "b"]);
  });

  it("selects nodes intersecting a drag box", () => {
    const nodes: Node[] = [{ node_id: "inside", name: "A", x: 100, y: 100 }, { node_id: "outside", name: "B", x: 400, y: 400 }];
    expect(selectionBoxIds(nodes, { x: 45, y: 65 }, { x: 155, y: 135 })).toEqual(["inside"]);
  });

  it("serializes the visible SVG map for screenshot export", () => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 100 100");
    const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
    rect.setAttribute("width", "50"); rect.setAttribute("height", "50");
    svg.append(rect); document.body.append(svg);
    const output = serializeSvgScreenshot(svg);
    expect(output).toContain("viewBox=\"0 0 100 100\"");
    expect(output).toContain("<rect");
    svg.remove();
  });
});
