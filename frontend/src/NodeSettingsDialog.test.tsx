import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

  it("edits and persists IOU node settings while showing the missing runtime state", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "i1", name: "IOU1", node_type: "iou", status: "stopped", runtime_supported: false, nvram: 256 }} open onOpenChange={vi.fn()} onSave={onSave} />);
    expect(screen.getByRole("status")).toHaveTextContent("IOU runtime is unavailable");
    fireEvent.change(screen.getByLabelText("NVRAM (KB)"), { target: { value: "512" } });
    fireEvent.change(screen.getByLabelText("Startup configuration"), { target: { value: "hostname IOU1" } });
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("i1", expect.objectContaining({ nvram: 512, startup_config: "hostname IOU1" })));
  });
});
