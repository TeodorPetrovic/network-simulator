import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import type { TopologySnapshot } from "./api";
import { useProjectHealth } from "./useProjectHealth";

vi.mock("./api", () => ({ api: { topology: vi.fn() } }));
const topology: TopologySnapshot = { project: { project_id: "p", name: "Lab" }, nodes: [{ node_id: "n", name: "Router", status: "started" }], links: [], drawings: [], event_cursor: 0 };
afterEach(() => { cleanup(); vi.useRealTimers(); vi.resetAllMocks(); });

describe("project health checks", () => {
  it("replaces a failed check with current health on the next periodic check", async () => {
    vi.useFakeTimers();
    vi.mocked(api.topology).mockRejectedValueOnce(new Error("Offline")).mockResolvedValue(topology);
    const projects = [topology.project];
    const { result } = renderHook(() => useProjectHealth(projects));
    await act(async () => {});
    expect(result.current.p.error).toContain("could not be checked");
    expect(result.current.p.health).toBeUndefined();
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(result.current.p.health?.label).toBe("Healthy");
    expect(result.current.p.error).toBeUndefined();
  });

  it("bounds concurrent requests and does not start more after unmount", async () => {
    const complete: Array<(value: TopologySnapshot) => void> = [];
    vi.mocked(api.topology).mockImplementation(() => new Promise((resolve) => complete.push(resolve)));
    const projects = Array.from({ length: 8 }, (_, index) => ({ project_id: `p${index}`, name: `Lab ${index}` }));
    const { unmount } = renderHook(() => useProjectHealth(projects));
    expect(api.topology).toHaveBeenCalledTimes(4);
    await act(async () => complete[0](topology));
    expect(api.topology).toHaveBeenCalledTimes(5);
    unmount();
    await act(async () => complete.forEach((resolve) => resolve(topology)));
    expect(api.topology).toHaveBeenCalledTimes(5);
  });

  it("ignores late responses when the project list changes", async () => {
    let finish!: (value: TopologySnapshot) => void;
    vi.mocked(api.topology).mockReturnValueOnce(new Promise((resolve) => { finish = resolve; })).mockResolvedValue(topology);
    const { result, rerender } = renderHook(({ projects }) => useProjectHealth(projects), { initialProps: { projects: [{ project_id: "old", name: "Old lab" }] } });
    rerender({ projects: [topology.project] });
    await act(async () => finish(topology));
    expect(result.current.old).toBeUndefined();
    expect(result.current.p.health?.label).toBe("Healthy");
  });
});
