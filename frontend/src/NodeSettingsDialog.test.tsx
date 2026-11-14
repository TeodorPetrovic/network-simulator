import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "./App.test-utils";
import { NodeSettingsDialog } from "./NodeSettingsDialog";

describe("NodeSettingsDialog", () => {
  it("disables adapter count for a running QEMU node with the stop hint", () => {
    render(<NodeSettingsDialog node={{ node_id: "q1", name: "Router", node_type: "qemu", status: "started", adapters: 2 }} open onOpenChange={vi.fn()} onSave={vi.fn()} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Network" }));
    expect(screen.getByLabelText("Network adapters")).toBeDisabled();
    expect(screen.getByText("Stop the device to change the number of adapters")).toBeInTheDocument();
  });

  it("leaves adapter count editable for a stopped Docker node", () => {
    render(<NodeSettingsDialog node={{ node_id: "d1", name: "Container", node_type: "docker", status: "stopped", adapters: 2 }} open onOpenChange={vi.fn()} onSave={vi.fn()} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Network" }));
    expect(screen.getByLabelText("Network adapters")).toBeEnabled();
  });

  it("edits and persists IOU node settings while showing the missing runtime state", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "i1", name: "IOU1", node_type: "iou", status: "stopped", runtime_supported: false, nvram: 256 }} open onOpenChange={vi.fn()} onSave={onSave} />);
    expect(screen.getByRole("status")).toHaveTextContent("IOU runtime is unavailable");
    fireEvent.change(screen.getByLabelText("NVRAM (KB)"), { target: { value: "512" } });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Startup" }));
    fireEvent.change(screen.getByLabelText("Startup configuration"), { target: { value: "hostname IOU1" } });
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("i1", expect.objectContaining({ nvram: 512, startup_config: "hostname IOU1" })));
  });
  it("retains edits across tabs and submits settings from every section", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "q1", name: "Router", node_type: "qemu", status: "stopped", hda_disk_image: "router.qcow2" }} open onOpenChange={vi.fn()} onSave={onSave} />);
    fireEvent.change(screen.getByLabelText("RAM (MB)"), { target: { value: "1024" } });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Storage" }));
    fireEvent.change(screen.getByLabelText("HDB disk image"), { target: { value: "data.qcow2" } });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Console" }));
    fireEvent.change(screen.getByLabelText("Export config command"), { target: { value: "/export" } });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "General" }));
    expect(screen.getByLabelText("RAM (MB)")).toHaveValue(1024);
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("q1", expect.objectContaining({ memory: 1024, hda_disk_image: "router.qcow2", hdb_disk_image: "data.qcow2", config_export_command: "/export" })));
  });

  it("keeps switch port rows aligned with their stored VLAN and EtherType", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "s1", name: "Switch", node_type: "ethernet_switch", status: "started", ports_mapping: [{ port_number: 0, name: "uplink", vlan: 20, type: "qinq", ethertype: "0x88a8" }, { port_number: 1, name: "client", vlan: 10, type: "access" }] }} open onOpenChange={vi.fn()} onSave={onSave} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Network" }));
    fireEvent.change(screen.getByLabelText("Port 2 VLAN"), { target: { value: "30" } });
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("s1", expect.objectContaining({ ports_mapping: [expect.objectContaining({ name: "uplink", vlan: 20, type: "qinq", ethertype: "0x88a8" }), expect.objectContaining({ name: "client", vlan: 30, type: "access" })] })));
  });

  it("resets a QinQ EtherType when changing a port to access mode", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "s1", name: "Switch", node_type: "ethernet_switch", status: "started", ports_mapping: [{ port_number: 0, name: "Ethernet0", vlan: 20, type: "qinq", ethertype: "0x88a8" }] }} open onOpenChange={vi.fn()} onSave={onSave} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Network" }));
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Port 1 mode" }), { key: "ArrowDown" });
    fireEvent.click(await screen.findByRole("option", { name: "access" }));
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("s1", expect.objectContaining({ ports_mapping: [expect.objectContaining({ type: "access", ethertype: "0x8100" })] })));
  });

  it("preserves an existing VPCS console choice when only its name changes", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(<NodeSettingsDialog node={{ node_id: "v1", name: "PC", node_type: "vpcs", status: "stopped", console_type: "ssh" }} open onOpenChange={vi.fn()} onSave={onSave} />);
    fireEvent.change(screen.getByLabelText("Device name"), { target: { value: "PC2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save device settings" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("v1", expect.objectContaining({ name: "PC2", console_type: "ssh" })));
  });

});
