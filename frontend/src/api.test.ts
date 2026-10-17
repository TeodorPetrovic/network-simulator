import { beforeEach, describe, expect, it, vi } from "vitest";
import { api, dockerConsoleUrl, dynamipsConsoleUrl, projectNotificationsUrl, qemuConsoleUrl, vpcsConsoleUrl } from "./api";

describe("GNS3 controller client", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });

  it("uses the selected server 3.1 /v3 routes and JSON create-project body", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => ({ project_id: "p1", name: "Core lab" }),
    } as Response);

    await api.createProject("Core lab");

    expect(fetchMock).toHaveBeenCalledWith("/v3/projects", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ name: "Core lab" }),
      headers: { "Content-Type": "application/json" },
    }));
  });

  it("searches the iShare2 metadata index and GNS3 .gns3a registry", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ source: "iShare2 / LabHub", results: [] }) } as Response);
    await api.searchIshareImages("c7200 router", "dynamips");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/marketplace/ishare2/images?q=c7200+router&kind=dynamips", expect.any(Object));

    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ source: "GNS3 Marketplace appliance registry", results: [] }) } as Response);
    await api.searchGns3Appliances("Cisco IOS");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/marketplace/gns3/appliances?q=Cisco+IOS", expect.any(Object));
  });

  it("starts a verified iShare2 download into the global runtime image library", async () => {
    const image = { id: 358, name: "vios-15.5.3M", image_type: "qemu", size: "122 MiB", total_size: 127926272, file_count: 1, catalog: "iShare2 / LabHub" };
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ id: image.id, name: image.name, image_type: image.image_type, files: ["vios-15.5.3M-virtioa.qcow2"], image_directory: "/global/images/qemu" }),
    } as Response);

    await api.downloadIshareImage(image);

    expect(fetch).toHaveBeenCalledWith("/v3/marketplace/ishare2/images/qemu/358/download", expect.objectContaining({
      method: "POST",
      body: "{}",
      headers: { "Content-Type": "application/json" },
    }));
  });

  it("streams a QEMU disk image to the global image library upload route", async () => {
    const file = new File([new Uint8Array([1, 2, 3])], "router.qcow2");
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ name: file.name, image: "/global/images/qemu/router.qcow2", template_type: "qemu", bytes: 3 }),
    } as Response);

    await api.uploadQemuImage(file);

    expect(fetch).toHaveBeenCalledWith("/v3/qemu/images/router.qcow2", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    }));
  });

  it("includes the consistent snapshot cursor when opening project notifications", () => {
    expect(projectNotificationsUrl("project-1", 42)).toBe(
      "ws://localhost:3000/v3/projects/project-1/notifications/ws?after=42",
    );
  });

  it("surfaces the controller message on failed requests", async () => {
    vi.mocked(fetch).mockResolvedValue({
      ok: false,
      status: 409,
      json: async () => ({ message: "Port is already in use" }),
    } as Response);

    await expect(api.listProjects()).rejects.toThrow("Port is already in use");
  });

  it("uses the GNS3 project template and node lifecycle routes", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ node_id: "n1", name: "Router A", status: "started" }),
    } as Response);

    await api.createNodeFromTemplate("p1", "t1", {
      name: "Router A",
      x: 40,
      y: 20,
      start_command: "/bin/sh -c 'sleep 3600'",
    });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/templates/t1", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ name: "Router A", x: 40, y: 20, start_command: "/bin/sh -c 'sleep 3600'" }),
    }));

    await api.startNode("p1", "n1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/nodes/n1/start", expect.objectContaining({
      method: "POST",
      body: "{}",
    }));
    await api.stopNode("p1", "n1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/nodes/n1/stop", expect.any(Object));
  });

  it("uses a same-origin WebSocket endpoint for a QEMU node", () => {
    const url = new URL(qemuConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/qemu/nodes/node-id/console/ws");
  });

  it("uses the GNS3 Docker console WebSocket endpoint", () => {
    const url = new URL(dockerConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/docker/nodes/node-id/console/ws");
  });

  it("uses the VPCS console WebSocket endpoint", () => {
    const url = new URL(vpcsConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/vpcs/nodes/node-id/console/ws");
  });

  it("uploads a Cisco IOS image as a raw file body", async () => {
    const file = new File([new Uint8Array([0x7f, 0x45, 0x4c, 0x46])], "c7200.bin");
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => ({ name: file.name, image: "/images/dynamips/c7200.bin", template_type: "dynamips", bytes: 4 }),
    } as Response);

    await api.uploadDynamipsImage(file);

    expect(fetch).toHaveBeenCalledWith("/v3/dynamips/images/c7200.bin", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    }));
  });

  it("uses the same-origin Dynamips IOS console endpoint", () => {
    const url = new URL(dynamipsConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/dynamips/nodes/node-id/console/ws");
  });
});
