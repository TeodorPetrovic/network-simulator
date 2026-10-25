import { afterEach, describe, expect, it as registerTest, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "../api";
import type { Link, Node } from "../api";
import { DrawingEditorDialog, HelpDialog, InfoDialog, InterfacePickerDialog, LinkPropertiesDialog, ScreenshotDialog, SnapshotsDialog, SymbolPickerDialog } from "./MapDialogs";
import { defaultEndpointLabelPosition } from "./helpers";

const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("project map dialogs", () => {
  it("places new endpoint labels beside their interface ports", () => {
    expect(defaultEndpointLabelPosition({ node_id: "q1", name: "Router", node_type: "qemu", adapters: 2 }, { node_id: "q1", adapter_number: 1, port_number: 0 })).toEqual({ x: 15, y: 31 });
  });

  it("lets a link creator pick an interface by its runtime port name", () => {
    const onPick = vi.fn();
    const node: Node = { node_id: "q1", name: "Router", node_type: "qemu", adapters: 2 };
    render(<InterfacePickerDialog node={node} links={[]} open onOpenChange={() => undefined} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: "Ethernet1" }));
    expect(onPick).toHaveBeenCalledWith({ node_id: "q1", adapter_number: 1, port_number: 0 });
  });

  it("searches the scrollable picker for high-numbered interfaces", () => {
    const onPick = vi.fn();
    const node: Node = { node_id: "q1", name: "Large QEMU", node_type: "qemu", adapters: 275 };
    render(<InterfacePickerDialog node={node} links={[]} open onOpenChange={() => undefined} onPick={onPick} />);

    fireEvent.change(screen.getByRole("textbox", { name: "Filter interfaces" }), { target: { value: "Ethernet274" } });
    fireEvent.click(screen.getByRole("button", { name: "Ethernet274" }));

    expect(onPick).toHaveBeenCalledWith({ node_id: "q1", adapter_number: 274, port_number: 0 });
  });

  it("disables interfaces already attached to another link", () => {
    const node: Node = { node_id: "q1", name: "Router", node_type: "qemu", adapters: 2 };
    render(<InterfacePickerDialog node={node} links={[{ link_id: "l1", nodes: [{ node_id: "q1", adapter_number: 0, port_number: 0 }] }]} open onOpenChange={() => undefined} onPick={() => undefined} />);
    expect(screen.getByRole("button", { name: "Ethernet0 · connected" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Ethernet1" })).toBeEnabled();
  });

  it("shows an unavailable state when snapshot routes have not landed", async () => {
    vi.spyOn(api, "snapshots").mockRejectedValue(new Error("Request failed (404)"));
    render(<SnapshotsDialog projectId="p1" open onOpenChange={() => undefined} onRestored={() => undefined} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Request failed (404)");
  });

  it("keeps link appearance available and reports unavailable filters and capture", async () => {
    vi.spyOn(api, "availableLinkFilters").mockRejectedValue(new Error("Request failed (404)"));
    vi.spyOn(api, "startLinkCapture").mockRejectedValue(new Error("Request failed (404)"));
    const link: Link = { link_id: "l1", nodes: [], link_type: "ethernet" };
    render(<LinkPropertiesDialog projectId="p1" link={link} nodes={[]} open onOpenChange={() => undefined} onUpdated={() => undefined} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Link filters are not available");
    expect(screen.queryByRole("button", { name: "Start in Web Wireshark" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Start capture" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Packet capture is not available"));
  });

  it("starts packet capture with Web Wireshark when enabled and Docker is available", async () => {
    vi.spyOn(api, "availableLinkFilters").mockResolvedValue([]);
    vi.spyOn(api, "settings").mockResolvedValue({ WebWireshark: { enabled: true } });
    vi.spyOn(api, "runtimes").mockResolvedValue({ docker: { available: true, version: "29", error: null } } as any);
    vi.spyOn(api, "startLinkCapture").mockResolvedValue({ link_id: "l1", nodes: [], capturing: true, wireshark: true });
    render(<LinkPropertiesDialog projectId="p1" link={{ link_id: "l1", nodes: [] }} nodes={[]} open onOpenChange={() => undefined} onUpdated={() => undefined} />);
    const checkbox = await screen.findByRole("checkbox", { name: "Web Wireshark" });
    expect(checkbox).toBeEnabled();
    fireEvent.click(checkbox);
    fireEvent.click(screen.getByRole("button", { name: "Start capture" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Starting Web Wireshark… (3-6 s)");
    await waitFor(() => expect(api.startLinkCapture).toHaveBeenCalledWith("p1", "l1", {
      capture_file_name: "l1.pcap", data_link_type: "DLT_EN10MB", wireshark: true,
    }));
  });

  it("saves editable text and position for both link endpoints", async () => {
    const link: Link = {
      link_id: "l1",
      nodes: [
        { node_id: "n1", adapter_number: 0, port_number: 0, label: { text: "Ethernet0", style: "fill: #111", x: 1, y: 2 } },
        { node_id: "n2", adapter_number: 0, port_number: 0, label: { text: "eth0", x: 3, y: 4 } },
      ],
    };
    const nodes: Node[] = [
      { node_id: "n1", name: "Router A", node_type: "qemu" },
      { node_id: "n2", name: "Router B", node_type: "qemu" },
    ];
    vi.spyOn(api, "availableLinkFilters").mockResolvedValue([]);
    vi.spyOn(api, "updateLink").mockResolvedValue(link);
    render(<LinkPropertiesDialog projectId="p1" link={link} nodes={nodes} open onOpenChange={() => undefined} onUpdated={() => undefined} />);
    fireEvent.change(screen.getAllByLabelText("Text", { selector: "input" })[0]!, { target: { value: "uplink" } });
    fireEvent.change(screen.getAllByLabelText("X", { selector: "input" })[0]!, { target: { value: "24" } });
    fireEvent.click(screen.getByRole("button", { name: "Save endpoint labels" }));
    await waitFor(() => expect(api.updateLink).toHaveBeenCalledWith("p1", "l1", {
      nodes: [
        { ...link.nodes[0], label: { text: "uplink", style: "fill: #111", x: 24, y: 2, rotation: 0 } },
        { ...link.nodes[1], label: { text: "eth0", x: 3, y: 4, rotation: 0 } },
      ],
    }));
  });

  it("shows an unavailable state when the symbol catalog is missing", async () => {
    vi.spyOn(api, "listSymbols").mockRejectedValue(new Error("Request failed (404)"));
    vi.spyOn(api, "defaultSymbols").mockResolvedValue({ default_symbol: "router.svg", symbols: {} });
    const node: Node = { node_id: "n1", name: "Router", node_type: "qemu" };
    render(<SymbolPickerDialog projectId="p1" node={node} open onOpenChange={() => undefined} onSaved={() => undefined} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Symbols are not available");
  });

  it("returns drawing editor values before placement and lists map shortcuts", () => {
    const onSave = vi.fn();
    render(<DrawingEditorDialog open onOpenChange={() => undefined} tool="text" mode="create" initial={null} onSave={onSave} />);
    fireEvent.change(screen.getByLabelText("Text"), { target: { value: "Core network" } });
    fireEvent.click(screen.getByRole("button", { name: "Choose appearance" }));
    expect(onSave).toHaveBeenCalledWith("text", expect.objectContaining({ text: "Core network" }));

    render(<HelpDialog open onOpenChange={() => undefined} />);
    expect(screen.getByText("Select devices in a region")).toBeInTheDocument();
    expect(screen.getByText("Ctrl / ⌘ + A")).toBeInTheDocument();

    render(<InfoDialog project={{ project_id: "p1", name: "Campus lab" }} nodeCount={2} linkCount={1} drawingCount={0} eventCursor={8} open onOpenChange={() => undefined} />);
    expect(screen.getByText("Campus lab")).toBeInTheDocument();
    expect(screen.getByText("#8")).toBeInTheDocument();
  });

  it("collects a screenshot name and lets the user choose SVG or PNG", () => {
    const onExport = vi.fn();
    render(<ScreenshotDialog open onOpenChange={() => undefined} onExport={onExport} pngAvailable />);
    fireEvent.change(screen.getByRole("textbox", { name: "Screenshot name" }), { target: { value: "Campus map" } });
    fireEvent.click(screen.getByRole("radio", { name: "PNG" }));
    fireEvent.click(screen.getByRole("button", { name: "Take a screenshot" }));
    expect(onExport).toHaveBeenCalledWith({ name: "Campus map", format: "png" });
  });
});
