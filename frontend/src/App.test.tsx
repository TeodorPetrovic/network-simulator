import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { routeTree } from "./routeTree.gen";
import { api } from "./api";
import type { Project, TopologySnapshot } from "./api";

vi.mock("./api", () => ({
  api: {
    listProjects: vi.fn(),
    createProject: vi.fn(),
    deleteProject: vi.fn(),
    openProject: vi.fn(),
    closeProject: vi.fn(),
    topology: vi.fn(),
    runtimes: vi.fn(),
    templates: vi.fn(),
    createNode: vi.fn(),
    createNodeFromTemplate: vi.fn(),
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
}));

const project: Project = { project_id: "p1", name: "Core lab", status: "opened" };
const emptyTopology: TopologySnapshot = {
  project,
  nodes: [],
  links: [],
  drawings: [],
  event_cursor: 0,
};

afterEach(cleanup);

class FakeWebSocket {
  static last: FakeWebSocket | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  close = vi.fn();

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

describe("project workspace migration", () => {
  beforeEach(() => {
    window.location.hash = "";
    vi.mocked(api.listProjects).mockResolvedValue([]);
    vi.mocked(api.createProject).mockResolvedValue(project);
    vi.mocked(api.openProject).mockResolvedValue(project);
    vi.mocked(api.topology).mockResolvedValue(emptyTopology);
    vi.mocked(api.runtimes).mockResolvedValue({
      docker: { available: true, version: "29.6.2", error: null },
      qemu: { available: false, version: null, qemu_img_available: false, image_directory: "images/qemu", image_count: 0, acceleration: "TCG", error: "QEMU unavailable" },
    });
    vi.mocked(api.templates).mockResolvedValue([]);
    vi.mocked(api.closeProject).mockResolvedValue(project);
    FakeWebSocket.last = null;
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  it("creates a project, opens it, and loads a snapshot-backed canvas", async () => {
    const router = renderRoutedApp();
    await screen.findByText("No projects yet");
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
    expect(document.querySelector(".stat-row b")).toHaveTextContent("1");

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
    vi.mocked(api.listProjects).mockResolvedValue([project]);
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
    renderRoutedApp();
    fireEvent.click(await screen.findByRole("button", { name: /Open project/ }));
    fireEvent.click(await screen.findByRole("button", { name: "＋ Add node" }));
    expect(await screen.findByRole("option", { name: "Docker · alpine:latest" })).toBeInTheDocument();
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
    vi.mocked(api.listProjects).mockResolvedValue([project]);
    renderRoutedApp();
    fireEvent.click(await screen.findByRole("button", { name: /Open project/ }));
    fireEvent.click(await screen.findByRole("button", { name: "＋ Add node" }));
    expect(await screen.findByText(/No local runtime images/)).toBeInTheDocument();
    expect(screen.queryByText(/placeholder/i)).not.toBeInTheDocument();
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
    vi.mocked(api.listProjects).mockResolvedValue([project]);
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
    renderRoutedApp();
    fireEvent.click(await screen.findByRole("button", { name: /Open project/ }));
    fireEvent.click(await screen.findByRole("button", { name: "＋ Add node" }));
    expect(await screen.findByRole("option", { name: "QEMU · router.qcow2" })).toBeInTheDocument();
    expect(screen.getByText("Creates a private copy-on-write disk. KVM is used when accessible; otherwise QEMU uses TCG.")).toBeInTheDocument();
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
});
