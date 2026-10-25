import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "./App.test-utils";
import { NodeSettingsDialog } from "./NodeSettingsDialog";

describe("NodeSettingsDialog", () => {
  it("disables adapter count for a running QEMU node with the stop hint", () => {
    render(<NodeSettingsDialog node={{ node_id: "q1", name: "Router", node_type: "qemu", status: "started", adapters: 2 }} open onOpenChange={vi.fn()} onSave={vi.fn()} />);
    expect(screen.getByLabelText("Network adapters")).toBeDisabled();
    expect(screen.getByText("Stop the device to change the number of adapters")).toBeInTheDocument();
  });

  it("leaves adapter count editable for a stopped Docker node", () => {
    render(<NodeSettingsDialog node={{ node_id: "d1", name: "Container", node_type: "docker", status: "stopped", adapters: 2 }} open onOpenChange={vi.fn()} onSave={vi.fn()} />);
    expect(screen.getByLabelText("Network adapters")).toBeEnabled();
  });
});
