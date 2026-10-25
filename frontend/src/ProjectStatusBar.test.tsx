import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TopologySnapshot } from "./api";
import { ProjectStatusBar } from "./ProjectStatusBar";

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
});
