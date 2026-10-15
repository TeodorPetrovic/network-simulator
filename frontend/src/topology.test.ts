import { describe, expect, it } from "vitest";
import type { TopologySnapshot } from "./api";
import { applyProjectEvent, sanitizeDrawingSvg } from "./topology";

const snapshot: TopologySnapshot = {
  project: { project_id: "p1", name: "Lab" },
  nodes: [{ node_id: "n1", name: "Router 1", x: 0, y: 0 }],
  links: [],
  drawings: [],
  event_cursor: 4,
};

describe("applyProjectEvent", () => {
  it("adds a created node once and advances the durable event cursor", () => {
    const event = {
      project_id: "p1",
      sequence: 6,
      action: "node.created",
      event: { node_id: "n2", name: "Switch 1", x: 90, y: 40 },
    };

    const first = applyProjectEvent(snapshot, event);
    const replay = applyProjectEvent(first, event);

    expect(first.nodes).toHaveLength(2);
    expect(replay.nodes).toHaveLength(2);
    expect(replay.event_cursor).toBe(6);
  });

  it("updates and removes the matching node without changing other collections", () => {
    const updated = applyProjectEvent(snapshot, {
      project_id: "p1",
      sequence: 5,
      action: "node.updated",
      event: { node_id: "n1", name: "Core router", x: 110, y: 70 },
    });
    const deleted = applyProjectEvent(updated, {
      project_id: "p1",
      sequence: 7,
      action: "node.deleted",
      event: { node_id: "n1" },
    });

    expect(updated.nodes[0]).toMatchObject({ name: "Core router", x: 110, y: 70 });
    expect(deleted.nodes).toEqual([]);
    expect(deleted.links).toEqual([]);
    expect(deleted.event_cursor).toBe(7);
  });

  it("ignores event kinds the current UI does not interpret", () => {
    const unchanged = applyProjectEvent(snapshot, {
      project_id: "p1",
      sequence: 9,
      action: "project.updated",
      event: { project_id: "p1", name: "Renamed" },
    });

    expect(unchanged).toBe(snapshot);
  });
});

describe("sanitizeDrawingSvg", () => {
  it("preserves supported drawing geometry and removes executable or external content", () => {
    const safe = sanitizeDrawingSvg(
      '<svg width="80" height="40"><rect x="1" y="2" width="70" height="30" fill="#fff" onclick="alert(1)"/><script>alert(2)</script><image href="https://example.invalid/a.png"/></svg>',
    );

    expect(safe).toContain("<rect");
    expect(safe).toContain('fill="#fff"');
    expect(safe).not.toMatch(/script|onclick|https:/i);
  });

  it("rejects malformed SVG input", () => {
    expect(sanitizeDrawingSvg("<svg><rect></svg>")).toBe("");
  });
});
