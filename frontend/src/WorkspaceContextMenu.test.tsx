import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Node } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { WorkspaceContextMenu } from "./WorkspaceContextMenu";
import { ContextMenu } from "@/components/ui/context-menu";

afterEach(cleanup);

function showFor(status: string, nodeType = "dynamips") {
  const node: Node = { node_id: "router", name: "R1", node_type: nodeType, status };
  const workspace = { topology: { links: [] }, connecting: false } as unknown as SimulatorWorkspaceProps;
  const common = { workspace, selectedNode: node, selectedLink: null, selectedDrawing: null, setConfigureNode: vi.fn(), setConfigNode: vi.fn(), setSymbolNode: vi.fn(), toggleNodeIsolation: vi.fn(), toggleNodeLock: vi.fn(), setLinkPropertiesOpen: vi.fn(), toggleLinkSuspend: vi.fn(), setDrawingEditTool: vi.fn(), setDrawingEditValues: vi.fn(), valuesFromDrawing: vi.fn(), setDrawingEditOpen: vi.fn(), changeDrawingOrder: vi.fn(), removeDrawing: vi.fn() };
  return render(<ContextMenu open><WorkspaceContextMenu {...common} /></ContextMenu>);
}

describe("Dynamips context menu lifecycle", () => {
  it("shows Start for stopped routers", () => {
    showFor("stopped");
    expect(screen.getByText("Start device")).toBeInTheDocument();
    expect(screen.queryByText("Stop device")).not.toBeInTheDocument();
  });

  it("shows Stop, Suspend, and Reload for started routers", () => {
    showFor("started");
    expect(screen.getByText("Stop device")).toBeInTheDocument();
    expect(screen.getByText("Suspend device")).toBeInTheDocument();
    expect(screen.getByText("Reload device")).toBeInTheDocument();
  });
});

describe("Startup config actions", () => {
  it("offers Edit config for VPCS and routers", () => {
    showFor("stopped", "vpcs");
    expect(screen.getByText("Edit config")).toBeInTheDocument();
  });

  it("hides config actions for Docker nodes", () => {
    showFor("stopped", "docker");
    expect(screen.queryByText("Edit config")).not.toBeInTheDocument();
  });
});
