import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { routeTree } from "./routeTree.gen";
import App from "./App";
import { api } from "./api";
import type { Project, TopologySnapshot } from "./api";

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    loadAddon() {}
    open() {}
    writeln() {}
    write() {}
    onData() { return { dispose() {} }; }
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

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
    searchGns3Appliances: vi.fn(),
    searchIshareImages: vi.fn(),
    downloadIshareImage: vi.fn(),
    gns3Appliance: vi.fn(),
    importGns3Appliance: vi.fn(),
    importedGns3Appliances: vi.fn(),
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
  },
  projectNotificationsUrl: vi.fn(() => "ws://localhost/notifications"),
  qemuConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/console`),
  dockerConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/docker-console`),
  vpcsConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/vpcs-console`),
  dynamipsConsoleUrl: vi.fn((projectId: string, nodeId: string) => `ws://localhost/${projectId}/${nodeId}/dynamips-console`),
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
    vi.mocked(api.closeProject).mockResolvedValue(project);
    FakeWebSocket.last = null;
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  it("creates a project, opens it, and loads a snapshot-backed canvas", async () => {
    const router = renderRoutedApp();
    await screen.findByRole("textbox", { name: "New project name" });
    fireEvent.change(screen.getByRole("textbox", { name: "New project name" }), {
      target: { value: "Core lab" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create project" }));

    expect(await screen.findByText("Your canvas is ready")).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe("/projects/p1"));
    expect(api.createProject).toHaveBeenCalledWith("Core lab");
    await waitFor(() => expect(api.openProject).toHaveBeenCalledWith("p1"));
    expect(api.topology).toHaveBeenCalledWith("p1");
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
    expect(screen.getByText("1 devices")).toBeInTheDocument();

    FakeWebSocket.last?.onmessage?.({
      data: JSON.stringify({
        project_id: "p1",
        sequence: 8,
        action: "node.created",
        event: { node_id: "n2", name: "Distribution switch", node_type: "ethernet_switch", x: 220, y: 100 },
      }),
    } as MessageEvent);
    expect(await screen.findByRole("button", { name: "Node Distribution switch" })).toBeInTheDocument();
  });

  it("shows live host metrics in the top-right resource monitor", async () => {
    renderWorkspace(null);
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

  it("opens an interactive serial console for a running QEMU node", async () => {
    vi.mocked(api.topology).mockResolvedValue({
      ...emptyTopology,
      nodes: [{ node_id: "vm-1", name: "Router VM", node_type: "qemu", status: "started", x: 0, y: 0 }],
    });
    renderWorkspace();

    fireEvent.click(await screen.findByRole("button", { name: "Node Router VM" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open console" }));

    expect(await screen.findByRole("region", { name: "Console for Router VM" })).toBeInTheDocument();
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

    expect(await screen.findByRole("region", { name: "Console for R1" })).toBeInTheDocument();
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

    expect(await screen.findByRole("region", { name: "Console for Alpine host" })).toBeInTheDocument();
    expect(screen.getByText("Docker attached console")).toBeInTheDocument();
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

  it("searches the GNS3 registry, shows separate image requirements, and imports the real appliance definition", async () => {
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
      required_files: [{ filename: "alpine-virt-3.19.1-x86_64.iso", version: "3.19.1", size: 10000000, md5sum: null, download_url: null }],
    };
    vi.mocked(api.searchGns3Appliances).mockResolvedValue([listing]);
    vi.mocked(api.gns3Appliance).mockResolvedValue(detail);
    vi.mocked(api.importGns3Appliance).mockResolvedValue({ appliance: { ...detail, imported: true }, matching_local_templates: 0, message: "Appliance definition imported; install or select its required image to use it" });
    renderWorkspace(null);
    fireEvent.click(await screen.findByRole("button", { name: "Image catalog" }));
    await screen.findByRole("textbox", { name: "Search catalog" });
    fireEvent.change(screen.getByRole("textbox", { name: "Search catalog" }), { target: { value: "alpine" } });
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    fireEvent.click(await screen.findByRole("button", { name: /Alpine Linux Virt/ }));

    expect(await screen.findByText("alpine-virt-3.19.1-x86_64.iso")).toBeInTheDocument();
    expect(screen.getByText(/required vendor images are supplied separately/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Import .gns3a definition" }));
    expect(await screen.findByText(/0 local runtime templates matched/)).toBeInTheDocument();
    expect(api.importGns3Appliance).toHaveBeenCalledWith("alpine-linux-virt.gns3a");
  });

  it("downloads supported iShare2 images into the global image library", async () => {
    const image = { id: 42, name: "alpine-virt", image_type: "qemu", size: "120 MB", total_size: 120000000, file_count: 1, catalog: "iShare2 / LabHub" };
    vi.mocked(api.searchIshareImages).mockResolvedValue([image]);
    vi.mocked(api.downloadIshareImage).mockResolvedValue({ id: 42, name: "alpine-virt", image_type: "qemu", files: ["alpine-virt-disk.qcow2"], image_directory: "/global/images/qemu" });
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

  it("creates a shared Ethernet segment with three selected nodes", async () => {
    const nodes = ["Edge A", "Edge B", "Edge C"].map((name, index) => ({
      node_id: `node-${index + 1}`,
      name,
      node_type: "docker",
      x: index * 120,
      y: 0,
    }));
    vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
    vi.mocked(api.createLink).mockResolvedValue({
      link_id: "segment-1",
      nodes: nodes.map((node, index) => ({ node_id: node.node_id, adapter_number: 0, port_number: index })),
    });
    renderWorkspace();
    fireEvent.click(await screen.findByRole("button", { name: "Connect ports" }));
    fireEvent.click(screen.getByRole("button", { name: "Edge A eth0" }));
    fireEvent.click(screen.getByRole("button", { name: "Edge B eth0" }));
    fireEvent.click(screen.getByRole("button", { name: "Edge C eth0" }));
    fireEvent.click(screen.getByRole("button", { name: "Create link · 3" }));

    await waitFor(() => expect(api.createLink).toHaveBeenCalledWith("p1", [
      { node_id: "node-1", adapter_number: 0, port_number: 0 },
      { node_id: "node-2", adapter_number: 0, port_number: 0 },
      { node_id: "node-3", adapter_number: 0, port_number: 0 },
    ]));
    expect(await screen.findByText("Ethernet segment created with 3 endpoints")).toBeInTheDocument();
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
