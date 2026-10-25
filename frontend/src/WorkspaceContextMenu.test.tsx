import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Node } from "./api";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { WorkspaceContextMenu } from "./WorkspaceContextMenu";
import { ContextMenu } from "@/components/ui/context-menu";

function showFor(status: string) {
  const node: Node = { node_id: "router", name: "R1", node_type: "dynamips", status };
  const workspace = { topology: { links: [] }, connecting: false } as unknown as SimulatorWorkspaceProps;
  const common = { workspace, selectedNode: node, selectedLink: null, selectedDrawing: null, setConfigureNode: vi.fn(), setSymbolNode: vi.fn(), toggleNodeIsolation: vi.fn(), toggleNodeLock: vi.fn(), setLinkPropertiesOpen: vi.fn(), toggleLinkSuspend: vi.fn(), setDrawingEditTool: vi.fn(), setDrawingEditValues: vi.fn(), valuesFromDrawing: vi.fn(), setDrawingEditOpen: vi.fn(), changeDrawingOrder: vi.fn(), removeDrawing: vi.fn() };
  return render(<ContextMenu><WorkspaceContextMenu {...common} /></ContextMenu>);
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
