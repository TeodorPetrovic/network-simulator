import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TopologySnapshot } from "./api";
import { ProjectStatusBar } from "./ProjectStatusBar";

afterEach(cleanup);

const topology: TopologySnapshot = { project: { project_id: "p1", name: "Lab" }, nodes: [{ node_id: "n1", name: "Router" }], links: [], drawings: [], event_cursor: 0 };

describe("ProjectStatusBar", () => {
  it("shows topology and controller state and opens the log with current counts", () => {
    const onOpenLog = vi.fn();
    render(<ProjectStatusBar topology={topology} projectLocked={false} connecting={false} linkEndpointCount={0} pendingDrawing={null} onCancelDrawing={vi.fn()} connection="connected" onOpenLog={onOpenLog} errors={2} warnings={1} resources={<button>Resources</button>} />);

    expect(screen.getByText("1 devices")).toBeInTheDocument();
    expect(screen.getByText("0 links")).toBeInTheDocument();
    expect(screen.getByText("Controller connected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open project log: 2 errors, 1 warnings" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open project log: 2 errors, 1 warnings" }));
    expect(onOpenLog).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Resources" })).toBeInTheDocument();
  });
  it("explains current issues and lets the user inspect the affected device", () => {
    const inspect = vi.fn();
    render(<ProjectStatusBar topology={{ ...topology, nodes: [{ node_id: "n1", name: "Router", status: "stopped", runtime_missing: true }] }} projectLocked={false} connecting={false} linkEndpointCount={0} pendingDrawing={null} onCancelDrawing={vi.fn()} connection="connected" onOpenLog={vi.fn()} errors={0} warnings={0} resources={null} onInspectNode={inspect} />);
    fireEvent.click(screen.getByRole("button", { name: "Project health: Needs attention" }));
    expect(screen.getByText(/Runtime data is missing/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Inspect Router" }));
    expect(inspect).toHaveBeenCalledWith("n1");
  });

  it("does not turn a healthy project red because of historical log errors", () => {
    render(<ProjectStatusBar topology={{ ...topology, nodes: [{ node_id: "n1", name: "Router", status: "started" }] }} projectLocked={false} connecting={false} linkEndpointCount={0} pendingDrawing={null} onCancelDrawing={vi.fn()} connection="connected" onOpenLog={vi.fn()} errors={3} warnings={1} resources={null} />);
    expect(screen.getByRole("button", { name: "Project health: Healthy" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open project log: 3 errors, 1 warnings" })).toBeInTheDocument();
  });

});
