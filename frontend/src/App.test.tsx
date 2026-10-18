import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it as registerTest, vi } from "vitest";
import { routeTree } from "./routeTree.gen";
import App from "./App";
import { ApplianceManagerPage } from "./ApplianceManagerPage";
import { api } from "./api";
import type { Gns3Appliance, LinkEndpoint, Node, Project, RuntimeTemplate, TopologySnapshot } from "./api";

const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

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

const project: Project = { project_id: "p1", name: "Core lab", status: "opened" };
const emptyTopology: TopologySnapshot = {
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

class FakeWebSocket {
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

function renderRoutedApp(initialPath = "/") {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [initialPath] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

function renderWorkspace(projectId: string | null = "p1") {
  render(<App projectId={projectId} navigateToProject={vi.fn()} />);
}

function ProjectNavigationHost() {
  const [projectId, setProjectId] = useState<string | null>(null);
  return <App projectId={projectId} navigateToProject={setProjectId} />;
}

describe("project workspace migration", () => {
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

  it("creates a project, opens it, and loads a snapshot-backed canvas", async () => {
    render(<ProjectNavigationHost />);
    await screen.findByRole("textbox", { name: "New project name" }, { timeout: 10_000 });
    fireEvent.change(screen.getByRole("textbox", { name: "New project name" }), {
      target: { value: "Core lab" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));

    expect(await screen.findByText("Your canvas is ready", {}, { timeout: 10_000 })).toBeInTheDocument();
    expect(api.createProject).toHaveBeenCalledWith("Core lab");
    await waitFor(() => expect(api.openProject).toHaveBeenCalledWith("p1"), { timeout: 10_000 });
    expect(api.topology).toHaveBeenCalledWith("p1");
  }, 15_000);

  it("opens the dedicated appliance manager route from the projects page", async () => {
    vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
    const router = renderRoutedApp();
    fireEvent.click(await screen.findByRole("button", { name: "Appliance manager" }));
    expect(await screen.findByRole("heading", { name: "Appliance library" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/appliances");
  });

  it("opens an existing project and shows its nodes from the topology snapshot", async () => {
    vi.mocked(api.listProjects).mockResolvedValue([project]);
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
    });
    renderRoutedApp();

    fireEvent.click(await screen.findByRole("button", { name: /Open project/ }));

    expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();

    act(() => {
      FakeWebSocket.last?.onmessage?.({
        data: JSON.stringify({
          project_id: "p1",
          sequence: 8,
          action: "node.created",
          event: { node_id: "n2", name: "Distribution switch", node_type: "ethernet_switch", x: 220, y: 100 },
        }),
      } as MessageEvent);
    });
    expect(await screen.findByRole("button", { name: "Node Distribution switch" }, { timeout: 10_000 })).toBeInTheDocument();
  });

  it("zooms to a selected device and resets the map scale", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "n1", name: "Edge router", node_type: "qemu", x: 180, y: 90 }],
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: "Node Edge router" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom to selection" }));
    expect(screen.getByText("300%")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByText("300%")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reset zoom" }));
    expect(screen.getAllByText("100%")).toHaveLength(2);
  });

  it("shows live host metrics from the topology map resource control", async () => {
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Resources/ }));

    expect(await screen.findByText("Resource monitor")).toBeInTheDocument();
    expect(screen.getByText("12.5%", { exact: true })).toBeInTheDocument();
    expect(screen.getByText("Controller RSS")).toBeInTheDocument();
  });

  it("starts every stopped device from the workspace toolbar", async () => {
    const nodes = [
      { node_id: "n1", name: "Edge A", node_type: "docker", status: "stopped", x: 0, y: 0 },
      { node_id: "n2", name: "Edge B", node_type: "docker", status: "started", x: 120, y: 0 },
    ];
    vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
    vi.mocked(api.startNode).mockResolvedValue({ ...nodes[0], status: "started" });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Start all" }));

    await waitFor(() => expect(api.startNode).toHaveBeenCalledTimes(1));
    expect(api.startNode).toHaveBeenCalledWith("p1", "n1");
  });

  it("auto-names a device added from the right-hand device library", async () => {
    const template = {
      template_id: "qemu-template-1",
      name: "router.qcow2",
      template_type: "qemu" as const,
      hda_disk_image: "/images/qemu/router.qcow2",
      adapters: 4,
      memory: 512,
      cpus: 1,
    };
    vi.mocked(api.templates).mockResolvedValue([template]);
    vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
      node_id: "vm-1",
      name: "router 1",
      node_type: "qemu",
      status: "stopped",
    });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Add router.qcow2" }));

    await waitFor(() => expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "qemu-template-1", {
      name: "router 1",
      x: -420,
      y: -220,
    }));
  });

  it("opens Docker and QEMU device groups when runtime templates finish loading", async () => {
    vi.mocked(api.templates).mockResolvedValue([
      { template_id: "docker-template", name: "alpine:latest", template_type: "docker", adapters: 2, image: "alpine:latest", image_digest: "sha256:test", memory: 512, cpus: 1 },
      { template_id: "qemu-template", name: "router.qcow2", template_type: "qemu", adapters: 4, hda_disk_image: "/images/router.qcow2", memory: 512, cpus: 1 },
    ]);
    renderWorkspace();

    expect(await screen.findByRole("button", { name: "Add alpine:latest" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Add router.qcow2" })).toBeInTheDocument();
    expect(screen.getByText("Docker")).toBeInTheDocument();
    expect(screen.getByText("QEMU / KVM")).toBeInTheDocument();
  });

  it("opens an interactive serial console for a running QEMU node", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "vm-1", name: "Router VM", node_type: "qemu", status: "started", x: 0, y: 0 }],
    });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Node Router VM" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open console" }));

    expect(await screen.findByRole("region", { name: "Console for Router VM" }, { timeout: 10_000 })).toBeInTheDocument();
    expect(screen.getByText("QEMU serial console")).toBeInTheDocument();
  });

  it("opens a console for a running Dynamips IOS router", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "ios-1", name: "R1", node_type: "dynamips", status: "started", x: 0, y: 0 }],
    });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Node R1" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open console" }));

    expect(await screen.findByRole("region", { name: "Console for R1" }, { timeout: 10_000 })).toBeInTheDocument();
    expect(screen.getByText("Dynamips IOS console")).toBeInTheDocument();
  });

  it("creates a Dynamips c7200 node with its IOS startup settings", async () => {
    const iosTemplate = {
      template_id: "ios-template",
      name: "c7200-adventerprisek9.bin · c7200",
      template_type: "dynamips" as const,
      platform: "c7200" as const,
      image: "/images/dynamips/c7200-adventerprisek9.bin",
      adapters: 1 as const,
      memory: 256,
      nvram: 128,
      idlepc: "",
    };
    vi.mocked(api.templates).mockResolvedValue([iosTemplate]);
    vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
      node_id: "ios-1",
      name: "R1",
      node_type: "dynamips",
      status: "stopped",
    });
    vi.mocked(api.topology).mockResolvedValue(emptyTopology);
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    fireEvent.change(await screen.findByRole("textbox", { name: "Node name" }), { target: { value: "R1" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Idle-PC address (optional)" }), { target: { value: "0x606e0538" } });
    fireEvent.change(screen.getByRole("textbox", { name: "IOS startup configuration (optional)" }), { target: { value: "hostname R1\nno ip domain-lookup" } });
    fireEvent.click(screen.getByRole("button", { name: "Create node" }));

    await waitFor(() => expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "ios-template", expect.objectContaining({
      name: "R1",
      idlepc: "0x606e0538",
      startup_config: "hostname R1\nno ip domain-lookup",
    })));
  });

  it("opens an attached terminal for a running Docker node", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "docker-1", name: "Alpine host", node_type: "docker", status: "started", x: 0, y: 0 }],
    });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Node Alpine host" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open console" }));

    expect(await screen.findByRole("region", { name: "Console for Alpine host" }, { timeout: 10_000 })).toBeInTheDocument();
    expect(screen.getByText("Docker attached console")).toBeInTheDocument();
  });

  it("opens the configured Docker auxiliary console through its WebSocket route", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "docker-aux", name: "Aux host", node_type: "docker", status: "started", aux_type: "telnet", aux_console_port: 5001, x: 0, y: 0 }],
    });
    renderWorkspace();

    fireEvent.contextMenu(await screen.findByRole("button", { name: "Node Aux host" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Open auxiliary console" }));

    expect(await screen.findByRole("region", { name: "Auxiliary console for Aux host" }, { timeout: 10_000 })).toBeInTheDocument();
    expect(screen.getByText("Docker auxiliary console")).toBeInTheDocument();
  });

  it("creates a node from an installed Docker image template", async () => {
    const template = {
      template_id: "template-1",
      name: "alpine:latest",
      template_type: "docker" as const,
      image: "alpine:latest",
      image_digest: "sha256:abcd",
      adapters: 8,
      memory: 0,
      cpus: 0,
    };
    vi.mocked(api.templates).mockResolvedValue([template]);
    vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
      node_id: "node-1",
      name: "Router A",
      node_type: "docker",
      image: "alpine:latest",
      status: "stopped",
      x: -420,
      y: -210,
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
      target: { value: "Router A" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Container command (optional)" }), {
      target: { value: "/bin/sh -c 'sleep 3600'" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create node" }));
    expect(await screen.findByRole("button", { name: "Node Router A" })).toBeInTheDocument();
    expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "template-1", {
      name: "Router A",
      x: -420,
      y: -210,
      start_command: "/bin/sh -c 'sleep 3600'",
    });
  });

  it("explains the real Docker image prerequisite when no image is installed", async () => {
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    expect(await screen.findByText(/No local templates/)).toBeInTheDocument();
    expect(screen.queryByText(/placeholder/i)).not.toBeInTheDocument();
  });

  it("shows local appliance image availability, suggests iShare2 matches, downloads one, and imports the definition", async () => {
    const listing = {
      file_name: "alpine-linux-virt.gns3a",
      name: "Alpine Linux Virt",
      registry_path: "appliances/alpine-linux-virt.gns3a",
      source_url: "https://github.com/GNS3/gns3-registry/blob/master/appliances/alpine-linux-virt.gns3a",
      download_url: "https://raw.githubusercontent.com/GNS3/gns3-registry/master/appliances/alpine-linux-virt.gns3a",
      imported: false,
    };
    const detail = {
      file_name: listing.file_name,
      appliance_id: "5899d1b7-672d-4e21-920d-908d254cb88c",
      name: "Alpine Linux Virt",
      category: "guest",
      description: "Small Alpine Linux virtual machine.",
      status: "stable",
      registry_version: 1,
      template_type: "qemu",
      imported: false,
      source_url: listing.source_url,
      required_files: [{ filename: "alpine-virt-3.19.1-x86_64.qcow2", version: "3.19.1", size: 10000000, md5sum: null, download_url: null }],
    };
    const suggestedImage = { id: 42, name: "Alpine virt 3.19.1", image_type: "qemu", size: "120 MB", total_size: 120000000, file_count: 1, files: ["alpine-virt-3.19.1-x86_64.qcow2"], catalog: "iShare2 / LabHub" };
    const localTemplate = { template_id: "alpine-qemu", name: "alpine-virt-3.19.1-x86_64.qcow2", template_type: "qemu" as const, hda_disk_image: "/global/images/qemu/alpine-virt-3.19.1-x86_64.qcow2", adapters: 8, memory: 512, cpus: 1 };
    vi.mocked(api.searchGns3Appliances).mockResolvedValue([listing]);
    vi.mocked(api.gns3Appliance).mockResolvedValue(detail);
    vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([localTemplate]);
    vi.mocked(api.searchIshareImages).mockResolvedValue([suggestedImage]);
    vi.mocked(api.downloadIshareImage).mockResolvedValue({ id: 42, name: suggestedImage.name, image_type: "qemu", files: ["alpine-virt-3.19.1-x86_64.qcow2"], source_files: ["alpine-virt-3.19.1-x86_64.qcow2"], image_directory: "/global/images/qemu" });
    vi.mocked(api.importGns3Appliance).mockResolvedValue({ appliance: { ...detail, imported: true }, matching_local_templates: 1, message: "Appliance imported and matched to installed runtime images" });
    renderWorkspace(null);
    fireEvent.click(await screen.findByRole("button", { name: "Image catalog" }));
    await screen.findByRole("textbox", { name: "Search catalog" }, { timeout: 15_000 });
    expect(screen.getByLabelText("GNS3 appliance results").closest('[data-slot="scroll-area"]')).toHaveClass("h-full");
    fireEvent.change(screen.getByRole("textbox", { name: "Search catalog" }), { target: { value: "alpine" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    fireEvent.click(await screen.findByRole("button", { name: /Alpine Linux Virt/ }));

    expect(await screen.findByText("alpine-virt-3.19.1-x86_64.qcow2")).toBeInTheDocument();
    expect(screen.getByText("Missing locally")).toBeInTheDocument();
    expect(await screen.findByText("Alpine virt 3.19.1")).toBeInTheDocument();
    expect(screen.getByText("Includes required file: alpine-virt-3.19.1-x86_64.qcow2")).toBeInTheDocument();
    expect(api.searchIshareImages).toHaveBeenCalledWith("alpine-virt-3.19.1-x86_64.qcow2", "qemu");
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText("Available locally")).toBeInTheDocument();
    expect(api.downloadIshareImage).toHaveBeenCalledWith(suggestedImage);
    expect(screen.getByText(/required vendor images are supplied separately/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Import .gns3a definition" }));
    expect(await screen.findByText(/1 local runtime template matched/)).toBeInTheDocument();
    expect(api.importGns3Appliance).toHaveBeenCalledWith("alpine-linux-virt.gns3a");
  }, 20_000);

  it("downloads supported iShare2 images into the global image library", async () => {
    const image = { id: 42, name: "alpine-virt", image_type: "qemu", size: "120 MB", total_size: 120000000, file_count: 1, files: ["alpine-virt-disk.qcow2"], catalog: "iShare2 / LabHub" };
    vi.mocked(api.searchIshareImages).mockResolvedValue([image]);
    vi.mocked(api.downloadIshareImage).mockResolvedValue({ id: 42, name: "alpine-virt", image_type: "qemu", files: ["alpine-virt-disk.qcow2"], source_files: ["alpine-virt-disk.qcow2"], image_directory: "/global/images/qemu" });
    renderWorkspace(null);
    fireEvent.click(await screen.findByRole("button", { name: "Image catalog" }));
    await screen.findByRole("textbox", { name: "Search catalog" });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "iShare2 images" }), { button: 0, ctrlKey: false });
    fireEvent.change(screen.getByRole("textbox", { name: "Search catalog" }), { target: { value: "alpine" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));

    await waitFor(() => expect(api.searchIshareImages).toHaveBeenCalledWith("alpine", "all"));
    expect(await screen.findByText("alpine-virt")).toBeInTheDocument();
    expect(screen.getByText(/downloaded into this controller’s global image library/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText(/Download complete: alpine-virt/)).toBeInTheDocument();
    expect(api.downloadIshareImage).toHaveBeenCalledWith(image);
    expect(api.searchIshareImages).toHaveBeenCalledWith("alpine", "all");
  });

  it("lists imported appliance configs and requires an explicit override for a mismatched image", async () => {
    const appliance = {
      file_name: "test-router.gns3a",
      appliance_id: "38aa3135-f169-4131-9b64-73605787b5ef",
      name: "Test Router",
      category: "router",
      description: "Router appliance with a versioned image slot.",
      status: "stable",
      registry_version: 8,
      template_type: "qemu",
      imported: true,
      source_url: "https://github.com/GNS3/gns3-registry",
      required_files: [],
      versions: [{
        name: "1.0",
        settings: "default",
        template_type: "qemu",
        runtime_supported: true,
        images: [{
          key: "hda_disk_image",
          filename: "official-router.qcow2",
          assignable: true,
          filesize: 20,
          md5sum: "expected-md5",
          assigned_image_id: null,
          assigned_image_name: null,
          suggested_image_id: "qemu-custom",
          suggested_image_name: "custom-router.qcow2",
          image_path: null,
          actual_size: 20,
          actual_md5sum: "actual-md5",
          actual_sha1sum: "actual-sha1",
          status: "mismatch",
          allow_checksum_mismatch: false,
        }],
      }],
    };
    const localImage = { image_id: "qemu-custom", name: "custom-router.qcow2", template_type: "qemu" as const, path: "/global/qemu/custom-router.qcow2", size: 20, md5sum: "actual-md5", sha1sum: "actual-sha1" };
    vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [appliance], local_images: [localImage], custom_image_options: [] });
    vi.mocked(api.setApplianceImageBinding).mockResolvedValue({ appliance });
    render(<ApplianceManagerPage onBack={vi.fn()} />);

    expect((await screen.findAllByText("Test Router")).length).toBeGreaterThan(0);
    expect(await screen.findByText("Checksum or size mismatch")).toBeInTheDocument();
    expect(screen.getByText("expected-md5")).toBeInTheDocument();
    expect(screen.getByText("actual-md5")).toBeInTheDocument();
    const override = screen.getByRole("checkbox", { name: /allow this image despite checksum or size mismatch/i });
    fireEvent.click(override);
    fireEvent.click(screen.getByRole("button", { name: "Assign image" }));

    await waitFor(() => expect(api.setApplianceImageBinding).toHaveBeenCalledWith("test-router.gns3a", {
      version: "1.0",
      image_key: "hda_disk_image",
      image_id: "qemu-custom",
      allow_checksum_mismatch: true,
    }));
  });

  it("keeps discovered images out of the appliance library until a custom appliance is created", async () => {
    const alpineTemplate: RuntimeTemplate = {
      template_id: "alpine-docker-template",
      name: "alpine:latest",
      template_type: "docker",
      image: "alpine:latest",
      image_digest: "sha256:alpine-image-digest",
      start_command: "",
      environment: "",
      extra_hosts: "",
      extra_volumes: [],
      adapters: 1,
      memory: 0,
      cpus: 0,
    };
    const mikrotikTemplate: RuntimeTemplate = {
      template_id: "mikrotik-qemu-template",
      name: "MikroTik CHR",
      template_type: "qemu",
      hda_disk_image: "/global/images/qemu/mikrotik-chr-7.23.8.qcow2",
      adapters: 2,
      memory: 256,
      cpus: 1,
      adapter_type: "virtio-net-pci",
      hda_disk_interface: "virtio",
      boot_priority: "c",
    };
    const manager = {
      appliances: [],
      local_images: [{ image_id: mikrotikTemplate.template_id, name: "mikrotik-chr-7.23.8.qcow2", template_type: "qemu" as const, path: mikrotikTemplate.hda_disk_image, size: 45_000_000, md5sum: null, sha1sum: null }],
      custom_image_options: [alpineTemplate, mikrotikTemplate],
    };
    const customAppliance: Gns3Appliance = {
      file_name: "custom-mikrotik.gns3a",
      appliance_id: "custom-mikrotik-id",
      name: "MikroTik CHR lab",
      origin: "custom",
      category: "guest",
      description: "Custom appliance created in the appliance library.",
      status: null,
      registry_version: 8,
      template_type: "qemu",
      imported: true,
      source_url: "Custom appliance created in this library",
      required_files: [],
      versions: [{
        name: "Default",
        settings: "default",
        template_type: "qemu",
        runtime_supported: true,
        images: [{
          key: "hda_disk_image", filename: "mikrotik-chr-7.23.8.qcow2", assignable: true,
          filesize: 45_000_000, md5sum: null, assigned_image_id: mikrotikTemplate.template_id,
          assigned_image_name: "mikrotik-chr-7.23.8.qcow2", suggested_image_id: mikrotikTemplate.template_id,
          suggested_image_name: "mikrotik-chr-7.23.8.qcow2", image_path: mikrotikTemplate.hda_disk_image,
          actual_size: 45_000_000, actual_md5sum: null, actual_sha1sum: null,
          status: "size-verified", allow_checksum_mismatch: false,
        }],
      }],
    };
    const createdState = { ...manager, appliances: [customAppliance] };
    const applianceTemplate = { ...mikrotikTemplate, template_id: "custom-mikrotik-project-template", name: "MikroTik CHR lab · Default", appliance_id: customAppliance.appliance_id, appliance_version: "Default" } as RuntimeTemplate;
    vi.mocked(api.applianceManager).mockResolvedValueOnce(manager).mockResolvedValueOnce(createdState);
    vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([applianceTemplate]);
    vi.mocked(api.createCustomAppliance).mockResolvedValue(customAppliance);
    render(<ApplianceManagerPage onBack={vi.fn()} />);

    await screen.findByRole("heading", { name: "Appliances" });
    expect(screen.queryByRole("button", { name: /alpine:latest/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /mikrotik-chr-7\.23\.8/ })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Create custom appliance" }));
    await screen.findByRole("dialog", { name: "Create custom appliance" });
    fireEvent.change(screen.getByRole("textbox", { name: "Appliance name" }), { target: { value: "MikroTik CHR lab" } });
    fireEvent.click(screen.getByRole("button", { name: "Create appliance" }));

    await waitFor(() => expect(api.createCustomAppliance).toHaveBeenCalledWith({
      name: "MikroTik CHR lab",
      source_template_id: mikrotikTemplate.template_id,
      runtime_settings: {},
    }));
    expect(await screen.findByRole("button", { name: /MikroTik CHR lab/ })).toBeInTheDocument();
    expect(screen.getByText("Custom appliance")).toBeInTheDocument();
  });

  it("opens the global image library modal before any appliance has been imported", async () => {
    vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
    render(<ApplianceManagerPage onBack={vi.fn()} />);

    expect(await screen.findByRole("button", { name: /Global images/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import .gns3a file" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add QEMU image" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Global images/ }));
    expect(await screen.findByRole("dialog", { name: "Global image library" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add QEMU image" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add Dynamips IOS image" })).toBeInTheDocument();
  });

  it("loads the GNS3 appliance catalog when its tab opens without requiring a search", async () => {
    const registryItem = {
      file_name: "alpine-linux-virt.gns3a",
      name: "Alpine Linux Virt",
      registry_path: "appliances/alpine-linux-virt.gns3a",
      source_url: "https://github.com/GNS3/gns3-registry/blob/master/appliances/alpine-linux-virt.gns3a",
      download_url: "https://raw.githubusercontent.com/GNS3/gns3-registry/master/appliances/alpine-linux-virt.gns3a",
      imported: false,
    };
    vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
    vi.mocked(api.searchGns3Appliances).mockResolvedValue([registryItem]);
    render(<ApplianceManagerPage onBack={vi.fn()} />);

    fireEvent.mouseDown(await screen.findByRole("tab", { name: /GNS3 registry/ }), { button: 0, ctrlKey: false });

    await waitFor(() => expect(api.searchGns3Appliances).toHaveBeenCalledWith(""));
    expect(await screen.findByRole("button", { name: /Alpine Linux Virt/ })).toBeInTheDocument();
  });

  it("creates a QEMU node from an installed disk image without a container command", async () => {
    const template = {
      template_id: "qemu-template-1",
      name: "router.qcow2",
      template_type: "qemu" as const,
      hda_disk_image: "/images/qemu/router.qcow2",
      adapters: 8,
      memory: 512,
      cpus: 1,
    };
    vi.mocked(api.templates).mockResolvedValue([template]);
    vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
      node_id: "vm-1",
      name: "Router VM",
      node_type: "qemu",
      status: "stopped",
      hda_disk_image: "/images/qemu/router.qcow2",
      x: -420,
      y: -210,
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
    expect(screen.getByText(/Creates a private copy-on-write disk/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
      target: { value: "Router VM" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create node" }));
    expect(await screen.findByRole("button", { name: "Node Router VM" })).toBeInTheDocument();
    expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "qemu-template-1", {
      name: "Router VM",
      x: -420,
      y: -210,
    });
  });

  it("uploads a custom QEMU image into the global library and selects its real template", async () => {
    const template = {
      template_id: "uploaded-qemu-template",
      name: "custom-router.qcow2",
      template_type: "qemu" as const,
      hda_disk_image: "/global/images/qemu/custom-router.qcow2",
      adapters: 8,
      memory: 512,
      cpus: 1,
    };
    vi.mocked(api.uploadQemuImage).mockResolvedValue({ name: "custom-router.qcow2", image: template.hda_disk_image, template_type: "qemu", bytes: 1024 });
    vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([]).mockResolvedValueOnce([template]);
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    const image = new File([new Uint8Array([1, 2, 3])], "custom-router.qcow2");
    fireEvent.change(screen.getByLabelText("Add a QEMU disk image"), { target: { files: [image] } });

    expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Node name" })).toHaveValue("custom-router");
    expect(await screen.findByText(/global image library/)).toBeInTheDocument();
    expect(api.uploadQemuImage).toHaveBeenCalledWith(image);
  });

  it("creates a VPCS node with its real startup script template field", async () => {
    const template = {
      template_id: "vpcs-template-1",
      name: "VPCS",
      template_type: "vpcs" as const,
      adapters: 1 as const,
      startup_script: "",
    };
    vi.mocked(api.templates).mockResolvedValue([template]);
    vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
      node_id: "pc-1",
      name: "PC A",
      node_type: "vpcs",
      status: "stopped",
      x: -420,
      y: -210,
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
    expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
      target: { value: "PC A" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Startup script (optional)" }), {
      target: { value: "ip 10.0.0.1/24" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create node" }));
    expect(await screen.findByRole("button", { name: "Node PC A" })).toBeInTheDocument();
    expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "vpcs-template-1", {
      name: "PC A",
      x: -420,
      y: -210,
      startup_script: "ip 10.0.0.1/24",
    });
  });

  it("creates a point-to-point Ethernet link between selected ports", async () => {
    const nodes = ["Edge A", "Edge B"].map((name, index) => ({
      node_id: `node-${index + 1}`,
      name,
      node_type: "docker",
      x: index * 120,
      y: 0,
    }));
    vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
    vi.mocked(api.createLink).mockResolvedValue({
      link_id: "link-1",
      nodes: nodes.map((node) => ({ node_id: node.node_id, adapter_number: 0, port_number: 0 })),
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: "Connect ports" }));
    fireEvent.click(screen.getByRole("button", { name: "Edge A eth0" }));
    fireEvent.click(screen.getByRole("button", { name: "Edge B eth0" }));
    fireEvent.click(screen.getByRole("button", { name: "Create link · 2" }));

    await waitFor(() => expect(api.createLink).toHaveBeenCalledWith("p1", [
      { node_id: "node-1", adapter_number: 0, port_number: 0 },
      { node_id: "node-2", adapter_number: 0, port_number: 0 },
    ]));
    expect(await screen.findByText("Ethernet segment created with 2 endpoints")).toBeInTheDocument();
  });

  it("exposes the complete node context-menu actions for a running device", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "n1", name: "Edge router", node_type: "qemu", status: "started", x: 0, y: 0 }],
    });
    renderWorkspace();
    fireEvent.contextMenu(await screen.findByRole("button", { name: "Node Edge router" }));

    for (const label of ["Suspend device", "Stop device", "Reload device", "Open console", "Edit device name", "Configure device", "Change symbol", "Isolate device", "Lock device", "Duplicate device", "Delete device"]) {
      expect(await screen.findByRole("menuitem", { name: label })).toBeInTheDocument();
    }
  });

  it("multi-selects devices and exposes bulk lifecycle actions", async () => {
    const nodes = ["Edge A", "Edge B"].map((name, index) => ({
      node_id: `node-${index + 1}`,
      name,
      node_type: "docker",
      status: "stopped",
      x: index * 120,
      y: 0,
    }));
    vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
    vi.mocked(api.startNode).mockImplementation(async (_projectId, nodeId) => ({ node_id: nodeId, name: nodeId === "node-1" ? "Edge A" : "Edge B", node_type: "docker", status: "started" }));
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: "Node Edge A" }), { ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Node Edge B" }), { ctrlKey: true });
    expect(screen.getByText("2 selected")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() => {
      expect(api.startNode).toHaveBeenCalledWith("p1", "node-1");
      expect(api.startNode).toHaveBeenCalledWith("p1", "node-2");
    });
  });

  it("redirects project links saved with the previous hash URL format", async () => {
    window.location.hash = "#/projects/p1";
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
    });
    const router = renderRoutedApp();

    expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/projects/p1");
  });

  it("restores a project from a direct typed-route URL", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
    });
    const router = renderRoutedApp("/projects/p1");

    expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/projects/p1");
    expect(api.openProject).toHaveBeenCalledWith("p1");
  });
});
