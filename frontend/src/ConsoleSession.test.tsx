import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";

const terminals: Array<{ data?: (value: string) => void; lines: string[] }> = [];
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    state: { data?: (value: string) => void; lines: string[] } = { lines: [] };
    constructor() { terminals.push(this.state); }
    loadAddon() {}
    open() {}
    focus() {}
    writeln(line: string) { this.state.lines.push(line); }
    write() {}
    onResize() { return { dispose() {} }; }
    onData(callback: (value: string) => void) { this.state.data = callback; return { dispose() {} }; }
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("./api", () => ({
  dockerConsoleUrl: () => Promise.resolve("ws://test/docker"),
  dockerAuxConsoleUrl: () => Promise.resolve("ws://test/aux"),
  dynamipsConsoleUrl: () => Promise.resolve("ws://test/dynamips"),
  qemuConsoleUrl: () => Promise.resolve("ws://test/qemu"),
  vpcsConsoleUrl: () => Promise.resolve("ws://test/vpcs"),
}));

import { CONSOLE_RECONNECT_HINT, ConsoleSession, type ConsoleTarget } from "./ConsoleDock";

class FakeSocket {
  static instances: FakeSocket[] = [];
  readyState = 0;
  sent: string[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) { FakeSocket.instances.push(this); }
  send(data: string) { this.sent.push(data); }
  close() { this.readyState = 3; }
  open() { this.readyState = 1; this.onopen?.(); }
  serverClose() { this.readyState = 3; this.onclose?.(); }
}

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });
const target: ConsoleTarget = { id: "n1:main", projectId: "p", nodeId: "n1", nodeName: "PC1", nodeType: "docker" };

beforeEach(() => {
  terminals.length = 0;
  FakeSocket.instances = [];
  vi.stubGlobal("WebSocket", Object.assign(FakeSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 }));
  vi.stubGlobal("requestAnimationFrame", (callback: () => void) => { callback(); return 0; });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("console session reconnect", () => {
  it("sends typed input as text frames and reconnects on Enter after the node stopped", async () => {
    render(<ConsoleSession target={target} active />);
    await flush();
    expect(FakeSocket.instances).toHaveLength(1);
    const first = FakeSocket.instances[0];
    first.open();
    expect(first.sent[0]).toBe('{"type":"resize","rows":24,"columns":80}');
    terminals[0].data?.("echo hi\r");
    expect(first.sent).toContain("echo hi\r");

    first.serverClose();
    expect(terminals[0].lines.at(-1)).toContain(CONSOLE_RECONNECT_HINT);
    terminals[0].data?.("x");
    await flush();
    expect(FakeSocket.instances).toHaveLength(1);
    terminals[0].data?.("\r");
    terminals[0].data?.("\r");
    await flush();
    expect(FakeSocket.instances).toHaveLength(2);
    FakeSocket.instances[1].open();
    terminals[0].data?.("echo again\r");
    expect(FakeSocket.instances[1].sent).toContain("echo again\r");
  });

  it("reconnects a closed session when the console is opened again, but keeps a live one", async () => {
    const view = render(<ConsoleSession target={target} active />);
    await flush();
    FakeSocket.instances[0].open();
    view.rerender(<ConsoleSession target={{ ...target, reconnect: 1 }} active />);
    await flush();
    expect(FakeSocket.instances).toHaveLength(1);
    FakeSocket.instances[0].serverClose();
    view.rerender(<ConsoleSession target={{ ...target, reconnect: 2 }} active />);
    await flush();
    expect(FakeSocket.instances).toHaveLength(2);
    expect(FakeSocket.instances[1].url).toBe("ws://test/docker");
  });
});
