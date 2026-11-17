import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WorkspaceDocks } from "./WorkspaceDocks";
import type { Link, TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";

const topology = { nodes: [{ node_id: "n1", name: "Router1", node_type: "dynamips", status: "started" }], links: [], drawings: [], event_cursor: 0 } as unknown as TopologySnapshot;
const makeWorkspace = () => ({ resources: null, selectedNode: null, onSelectNode: vi.fn(), onNodeContext: vi.fn() }) as unknown as SimulatorWorkspaceProps;

describe("WorkspaceDocks", () => {
  afterEach(() => {
    cleanup();
  });

  it("shows topology and server summaries and selects a topology node", () => {
    const workspace = makeWorkspace();
    render(<WorkspaceDocks workspace={workspace} topology={topology} topologyOpen serversOpen onCloseTopology={vi.fn()} onCloseServers={vi.fn()} selectedLink={null} onConfigureNode={vi.fn()} onOpenLinkProperties={vi.fn()} onToggleLinkSuspend={vi.fn()} onOpenCaptureViewer={vi.fn()} renderNodeContextMenu={() => null} />);
    fireEvent.click(screen.getByRole("button", { name: /Router1/ }));
    expect(workspace.onSelectNode).toHaveBeenCalledWith("n1");
    expect(screen.getByLabelText("Servers Summary")).toBeInTheDocument();
    expect(screen.getByLabelText("Topology Summary")).toBeInTheDocument();
  });

  it("shows a Capture button in the selected-link inspector", () => {
    const link = { link_id: "l1", suspend: false } as unknown as Link;
    render(<WorkspaceDocks workspace={makeWorkspace()} topology={topology} topologyOpen serversOpen={false} onCloseTopology={vi.fn()} onCloseServers={vi.fn()} selectedLink={link} onConfigureNode={vi.fn()} onOpenLinkProperties={vi.fn()} onToggleLinkSuspend={vi.fn()} onOpenCaptureViewer={vi.fn()} renderNodeContextMenu={() => null} />);
    expect(screen.getByRole("button", { name: "Capture" })).toBeInTheDocument();
  });

  it("calls the capture callback with the selected link when Capture is clicked", () => {
    const link = { link_id: "l1", suspend: false } as unknown as Link;
    const onOpenCaptureViewer = vi.fn();
    render(<WorkspaceDocks workspace={makeWorkspace()} topology={topology} topologyOpen serversOpen={false} onCloseTopology={vi.fn()} onCloseServers={vi.fn()} selectedLink={link} onConfigureNode={vi.fn()} onOpenLinkProperties={vi.fn()} onToggleLinkSuspend={vi.fn()} onOpenCaptureViewer={onOpenCaptureViewer} renderNodeContextMenu={() => null} />);
    fireEvent.click(screen.getByRole("button", { name: "Capture" }));
    expect(onOpenCaptureViewer).toHaveBeenCalledWith(link);
  });

});
