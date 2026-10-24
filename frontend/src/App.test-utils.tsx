import { cleanup, render } from "@testing-library/react";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { useState } from "react";
import { afterEach, beforeEach, it as registerTest, vi } from "vitest";
import { routeTree } from "./routeTree.gen";
import App from "./App";
import { api } from "./api";
import type { LinkEndpoint, Node, Project, TopologySnapshot } from "./api";

export const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    loadAddon() {}
    open() {}
    writeln() {}
    write() {}
    onData() { return { dispose() {} }; }
    onResize() { return { dispose() {} }; }
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

vi.mock("./map/MapDialogs", async () => {
  const React = await import("react");
  return {
    InterfacePickerDialog: ({ node, open, onOpenChange, onPick }: { node: Node | null; open: boolean; onOpenChange: (open: boolean) => void; onPick: (endpoint: LinkEndpoint) => void }) => node && open
      ? React.createElement("button", {
        onClick: () => {
          onPick({ node_id: node.node_id, adapter_number: 0, port_number: 0 });
          onOpenChange(false);
        },
      }, `${node.name} ${node.node_type === "qemu" ? "Ethernet0" : "eth0"}`)
      : null,
    DrawingEditorDialog: () => null,
    SnapshotsDialog: () => null,
    LinkPropertiesDialog: () => null,
    SymbolPickerDialog: () => null,
    LogConsoleDialog: () => null,
    HelpDialog: () => null,
    InfoDialog: () => null,
    ScreenshotDialog: () => null,
  };
});

vi.mock("./api", () => ({
  api: {
    listProjects: vi.fn(),
    createProject: vi.fn(),
    deleteProject: vi.fn(),
    openProject: vi.fn(),
    closeProject: vi.fn(),
    topology: vi.fn(),
    runtimes: vi.fn(),
    resourceMonitor: vi.fn(),
    templates: vi.fn(),
    updateTemplate: vi.fn(),
    listSymbols: vi.fn(),
    defaultSymbols: vi.fn(),
    searchGns3Appliances: vi.fn(),
    searchIshareImages: vi.fn(),
    downloadIshareImage: vi.fn(),
    gns3Appliance: vi.fn(),
    importGns3Appliance: vi.fn(),
    importedGns3Appliances: vi.fn(),
    applianceManager: vi.fn(),
    createCustomAppliance: vi.fn(),
    setApplianceImageBinding: vi.fn(),
    clearApplianceImageBinding: vi.fn(),
    importGns3ApplianceFile: vi.fn(),
    createNode: vi.fn(),
    createNodeFromTemplate: vi.fn(),
    uploadDynamipsImage: vi.fn(),
    uploadQemuImage: vi.fn(),
    updateNode: vi.fn(),
    deleteNode: vi.fn(),
    startNode: vi.fn(),
    stopNode: vi.fn(),
    suspendNode: vi.fn(),
    reloadNode: vi.fn(),
    createLink: vi.fn(),
    deleteLink: vi.fn(),
    updateLink: vi.fn(),
  },
  projectNotificationsUrl: vi.fn(() => "ws://localhost/notifications"),
  qemuConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/console`),
  dockerConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/docker-console`),
  dockerAuxConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/docker-aux-console`),
  vpcsConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/vpcs-console`),
  dynamipsConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/dynamips-console`),
  dockerHttpConsoleUrl: vi.fn((protocol: string, host: string, port: number, path: string) => `${protocol}://${host}:${port}${path}`),
  symbolRawUrl: vi.fn((symbolId: string) => `/v3/symbols/${encodeURIComponent(symbolId)}/raw`),
}));

export const project: Project = { project_id: "p1", name: "Core lab", status: "opened" };
export const emptyTopology: TopologySnapshot = {
  project,
  nodes: [],
  links: [],
  drawings: [],
  event_cursor: 0,
};

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

export class FakeWebSocket {
  static OPEN = 1;
  static last: FakeWebSocket | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  readyState = 1;
  close = vi.fn();
  send = vi.fn();

  constructor(_url: string) {
    FakeWebSocket.last = this;
  }
}

export function renderRoutedApp(initialPath = "/") {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [initialPath] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

export function renderWorkspace(projectId: string | null = "p1") {
  render(<App projectId={projectId} navigateToProject={vi.fn()} />);
}

export function ProjectNavigationHost() {
  const [projectId, setProjectId] = useState<string | null>(null);
  return <App projectId={projectId} navigateToProject={setProjectId} />;
}


beforeEach(() => {
  window.location.hash = "";
  vi.mocked(api.listProjects).mockResolvedValue([]);
  vi.mocked(api.createProject).mockResolvedValue(project);
  vi.mocked(api.openProject).mockResolvedValue(project);
  vi.mocked(api.topology).mockResolvedValue(emptyTopology);
  vi.mocked(api.runtimes).mockResolvedValue({
    docker: { available: true, version: "29.6.2", error: null },
    qemu: { available: false, version: null, qemu_img_available: false, image_directory: "images/qemu", image_count: 0, acceleration: "TCG", kvm_accessible: false, acceleration_error: "KVM unavailable", error: "QEMU unavailable" },
    vpcs: { available: false, version: null, error: "VPCS unavailable" },
    dynamips: { available: false, version: null, binary: "dynamips", image_directory: "images/dynamips", image_count: 0, error: "Dynamips unavailable" },
    ubridge: { available: false, binary: "ubridge", version: null, error: "uBridge unavailable" },
  });
  vi.mocked(api.resourceMonitor).mockResolvedValue({
    host: { cpu_usage_percent: 12.5, cpu_count: 8, memory_total_bytes: 16_000_000_000, memory_used_bytes: 8_000_000_000, load_average: [0.2, 0.3, 0.4] },
    controller: { memory_rss_bytes: 100_000_000, thread_count: 12 },
    sampled_at_unix_ms: 1_797_000_000_000,
  });
  vi.mocked(api.templates).mockResolvedValue([]);
  vi.mocked(api.listSymbols).mockResolvedValue([]);
  vi.mocked(api.defaultSymbols).mockResolvedValue({ default_symbol: ":/symbols/router.svg", symbols: { router: "router.svg", pc: "pc.svg", switch: "switch.svg", cloud: "cloud.svg" } });
  vi.mocked(api.closeProject).mockResolvedValue(project);
  FakeWebSocket.last = null;
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

export { api } from "./api";
