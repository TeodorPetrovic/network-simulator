import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceDocks } from "./WorkspaceDocks";
import type { TopologySnapshot } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";

const topology = { nodes: [{ node_id: "n1", name: "Router1", node_type: "dynamips", status: "started" }], links: [], drawings: [], event_cursor: 0 } as unknown as TopologySnapshot;
const makeWorkspace = () => ({ resources: null, selectedNode: null, onSelectNode: vi.fn(), onNodeContext: vi.fn() }) as unknown as SimulatorWorkspaceProps;

describe("WorkspaceDocks", () => {
  it("shows topology and server summaries and selects a topology node", () => {
    const workspace = makeWorkspace();
    render(<WorkspaceDocks workspace={workspace} topology={topology} topologyOpen serversOpen consoleOpen onCloseTopology={vi.fn()} onCloseServers={vi.fn()} onCloseConsole={vi.fn()} onOpenConsole={vi.fn()} selectedLink={null} onConfigureNode={vi.fn()} onOpenLinkProperties={vi.fn()} onToggleLinkSuspend={vi.fn()} renderNodeContextMenu={() => null} />);
    fireEvent.click(screen.getByRole("button", { name: /Router1/ }));
    expect(workspace.onSelectNode).toHaveBeenCalledWith("n1");
    expect(screen.getByLabelText("Servers Summary")).toBeInTheDocument();
    expect(screen.getByLabelText("Topology Summary")).toBeInTheDocument();
  });

  it("offers a way to reopen the console dock when it is closed", () => {
    const onOpenConsole = vi.fn();
    render(<WorkspaceDocks workspace={makeWorkspace()} topology={topology} topologyOpen={false} serversOpen={false} consoleOpen={false} onCloseTopology={vi.fn()} onCloseServers={vi.fn()} onCloseConsole={vi.fn()} onOpenConsole={onOpenConsole} selectedLink={null} onConfigureNode={vi.fn()} onOpenLinkProperties={vi.fn()} onToggleLinkSuspend={vi.fn()} renderNodeContextMenu={() => null} />);
    fireEvent.click(screen.getByRole("button", { name: "Show Console" }));
    expect(onOpenConsole).toHaveBeenCalledOnce();
  });
});
