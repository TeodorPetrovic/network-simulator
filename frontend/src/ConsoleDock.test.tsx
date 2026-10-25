import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConsoleDock, dockerResizeControlFrame, sendDockerResizeControl } from "./ConsoleDock";

vi.mock("@xterm/xterm", () => ({ Terminal: class { rows=24; cols=80; loadAddon(){} open(){} writeln(){} write(){} onResize(){return {dispose(){}}} onData(){return {dispose(){}}} dispose(){} } }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit(){} } }));

beforeEach(() => localStorage.clear());

describe("floating console dock", () => {
  const props = (minimized = false) => ({ targets: [{ id: "n1", projectId: "p", nodeId: "n1", nodeName: "Router1", nodeType: "vpcs" as const }], activeNodeId: "n1", minimized, onSelect: vi.fn(), onMinimize: vi.fn(), onRestore: vi.fn(), onHide: vi.fn(), onClose: vi.fn() });
  it("restores a requested device console and invokes selection", () => {
    const p = props(true);
    render(<ConsoleDock {...p} />);
    fireEvent.click(screen.getByRole("tab", { name: "Router1" }));
    expect(p.onSelect).toHaveBeenCalledWith("n1");
    expect(p.onRestore).toHaveBeenCalledOnce();
  });
  it("minimizes and restores the floating panel", () => {
    const p = props();
    render(<ConsoleDock {...p} />);
    fireEvent.click(screen.getByRole("button", { name: "Minimize console dock" }));
    expect(p.onMinimize).toHaveBeenCalledOnce();
    expect(JSON.parse(localStorage.getItem("ns.console.float")!).minimized).toBe(true);
    const minimized = props(true);
    render(<ConsoleDock {...minimized} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Restore console dock" }).at(-1)!);
    expect(minimized.onRestore).toHaveBeenCalledOnce();
  });
});

describe("Docker console resize control frame", () => {
  it("matches the text frame accepted by the Docker console WebSocket", () => {
    expect(dockerResizeControlFrame(24, 80)).toBe('{"type":"resize","rows":24,"columns":80}');
  });

  it("sends a resize only for positive dimensions on an open socket", () => {
    const send = vi.fn();
    const socket = { readyState: WebSocket.OPEN, send };

    expect(sendDockerResizeControl(socket, 40, 120)).toBe(true);
    expect(send).toHaveBeenCalledWith('{"type":"resize","rows":40,"columns":120}');
    expect(sendDockerResizeControl(socket, 0, 120)).toBe(false);
    expect(sendDockerResizeControl({ ...socket, readyState: WebSocket.CONNECTING }, 40, 120)).toBe(false);
    expect(send).toHaveBeenCalledTimes(1);
  });
});
