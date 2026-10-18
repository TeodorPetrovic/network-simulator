import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it as registerTest, vi } from "vitest";
import { api } from "./api";
import type { TopologySnapshot } from "./api";
import { BuiltinNodeEditorPage } from "./BuiltinNodeEditorPage";

const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("./PagesShell", async () => {
  const React = await import("react");
  return {
    PagesShell: ({ children }: { children: import("react").ReactNode }) => React.createElement("main", null, children),
    PageTitle: ({ title }: { title: string }) => React.createElement("h1", null, title),
  };
});
vi.mock("./api", () => ({ api: { listProjects: vi.fn(), topology: vi.fn(), configureNode: vi.fn() } }));

const project = { project_id: "project-1", name: "Core lab" };
const topology: TopologySnapshot = {
  project,
  nodes: [{
    node_id: "switch-1",
    name: "SW1",
    node_type: "ethernet_switch",
    status: "started",
    ports_mapping: [
      { name: "Ethernet0", port_number: 0, type: "access", vlan: 10, ethertype: "0x8100" },
      { name: "Ethernet1", port_number: 1, type: "dot1q", vlan: 20, ethertype: "0x8100" },
    ],
    console_type: "none",
  }],
  links: [],
  drawings: [],
  event_cursor: 0,
};

afterEach(cleanup);
beforeEach(() => {
  Object.defineProperty(Element.prototype, "scrollIntoView", { configurable: true, value: () => undefined });
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", { configurable: true, value: () => false });
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", { configurable: true, value: () => undefined });
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", { configurable: true, value: () => undefined });
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  vi.mocked(api.topology).mockResolvedValue(topology);
  vi.mocked(api.configureNode).mockResolvedValue(topology.nodes[0]);
});

describe("BuiltinNodeEditorPage", () => {
  it("submits switch configuration as flat node fields", async () => {
    render(<BuiltinNodeEditorPage nodeType="ethernet_switch" />);
    expect(await screen.findByLabelText("Node")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText("Node")).not.toBeDisabled());

    fireEvent.pointerDown(screen.getByLabelText("Node"), { button: 0, ctrlKey: false, pointerType: "mouse" });
    fireEvent.click(await screen.findByRole("option", { name: "SW1" }, { timeout: 10_000 }));
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "SW1-renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => expect(api.configureNode).toHaveBeenCalledWith("project-1", "switch-1", {
      name: "SW1-renamed",
      console_type: "none",
      ports_mapping: [
        { port_number: 0, vlan: 10, type: "access", ethertype: "0x8100" },
        { port_number: 1, vlan: 20, type: "dot1q", ethertype: "0x8100" },
      ],
    }));
  }, 15_000);
});
