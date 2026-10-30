import { beforeEach, describe, expect, it, vi } from "vitest";
import { api, dockerAuxConsoleUrl, dockerConsoleUrl, dockerHttpConsoleUrl, dynamipsConsoleUrl, projectNotificationsUrl, qemuConsoleUrl, symbolRawUrl, vpcsConsoleUrl, vncConsoleUrl } from "./api";

describe("GNS3 controller client", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
    sessionStorage.removeItem("network-simulator.access-token");
    localStorage.removeItem("network-simulator.access-token");
  });

  it("submits OAuth2 password form data and attaches the stored bearer token", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ access_token: "signed.jwt.token", token_type: "bearer" }) } as Response);
    await api.login("admin", "correct-horse-battery-staple");
    expect(fetchMock).toHaveBeenNthCalledWith(1, "/v3/access/users/login", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: expect.any(URLSearchParams),
    }));
    expect(sessionStorage.getItem("network-simulator.access-token")).toBe("signed.jwt.token");

    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({}) } as Response);
    await api.runtimes();
    expect(fetchMock).toHaveBeenNthCalledWith(2, "/v3/runtimes", expect.objectContaining({ headers: { Authorization: "Bearer signed.jwt.token" } }));
  });

  it("requests a short-lived WebSocket ticket with the bearer header", async () => {
    sessionStorage.setItem("network-simulator.access-token", "signed.jwt.token");
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "opaque-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await projectNotificationsUrl("project-1", 12));
    expect(url.searchParams.get("ticket")).toBe("opaque-ticket");
    expect(url.searchParams.has("token")).toBe(false);
    expect(url.searchParams.get("after")).toBe("12");
    expect(fetch).toHaveBeenCalledWith("/v3/access/ws-ticket", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer signed.jwt.token" },
      body: JSON.stringify({ path: "/v3/projects/project-1/notifications/ws" }),
    }));
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

  it("loads appliance configs and persists explicit image bindings per version and image slot", async () => {
    const fetchMock = vi.mocked(fetch);
    const manager = { appliances: [], local_images: [], custom_image_options: [] };
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => manager } as Response);
    await api.applianceManager();
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/marketplace/gns3/manager", expect.any(Object));

    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ name: "Alpine custom" }) } as Response);
    await api.createCustomAppliance({ name: "Alpine custom", source_template_id: "docker-image-id" });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/marketplace/gns3/custom", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ name: "Alpine custom", source_template_id: "docker-image-id" }),
    }));

    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ appliance: {} }) } as Response);
    await api.setApplianceImageBinding("router.gns3a", { version: "15.2", image_key: "hda_disk_image", image_id: "qemu-1", allow_checksum_mismatch: true });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/marketplace/gns3/appliances/router.gns3a/bindings", expect.objectContaining({
      method: "PUT",
      body: JSON.stringify({ version: "15.2", image_key: "hda_disk_image", image_id: "qemu-1", allow_checksum_mismatch: true }),
    }));
  });

  it("uploads a local GNS3 appliance definition to the global appliance library", async () => {
    const file = new File([JSON.stringify({ appliance_id: "id" })], "router.gns3a", { type: "application/json" });
    vi.mocked(fetch).mockResolvedValue({ ok: true, status: 200, json: async () => ({ file_name: file.name }) } as Response);
    await api.importGns3ApplianceFile(file);
    expect(fetch).toHaveBeenCalledWith("/v3/marketplace/gns3/import/router.gns3a", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    }));
  });

  it("starts a verified iShare2 download into the global runtime image library", async () => {
    const image = { id: 358, name: "vios-15.5.3M", image_type: "qemu", size: "122 MiB", total_size: 127926272, file_count: 1, files: ["virtioa.qcow2"], catalog: "iShare2 / LabHub" };
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ id: image.id, name: image.name, image_type: image.image_type, files: ["vios-15.5.3M-virtioa.qcow2"], source_files: ["virtioa.qcow2"], image_directory: "/global/images/qemu" }),
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

  it("includes the consistent snapshot cursor when opening project notifications", async () => {
    sessionStorage.setItem("network-simulator.access-token", "session-token");
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await projectNotificationsUrl("project-1", 42));
    expect(url.toString()).toBe("ws://localhost:3000/v3/projects/project-1/notifications/ws?after=42&ticket=test-ticket");
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

  it("uses a same-origin WebSocket endpoint for a QEMU node", async () => {
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await qemuConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/qemu/nodes/node-id/console/ws");
  });

  it("uses the GNS3 Docker console WebSocket endpoint", async () => {
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await dockerConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/docker/nodes/node-id/console/ws");
  });

  it("uses the registered Docker auxiliary console WebSocket endpoint", async () => {
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await dockerAuxConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/docker/nodes/node-id/aux_console/ws");
  });

  it("uses the controller VNC proxy for QEMU and Docker", async () => {
    vi.mocked(fetch).mockResolvedValue({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const qemu = new URL(await vncConsoleUrl("qemu", "project-id", "node-id"));
    const docker = new URL(await vncConsoleUrl("docker", "project-id", "node-id"));
    expect(qemu.pathname).toBe("/v3/compute/projects/project-id/qemu/nodes/node-id/vnc/ws");
    expect(docker.pathname).toBe("/v3/compute/projects/project-id/docker/nodes/node-id/vnc/ws");
  });

  it("builds browser URLs for Docker HTTP(S) consoles and encodes symbol raw paths", () => {
    expect(dockerHttpConsoleUrl("https", "127.0.0.1", 8080, "/vnc.html"))
      .toBe("https://127.0.0.1:8080/vnc.html");
    expect(dockerHttpConsoleUrl("http", "localhost", 0, "/")).toBeNull();
    expect(symbolRawUrl("custom router.svg")).toBe("/v3/symbols/custom%20router.svg/raw");
  });

  it("fetches custom symbol data with the session bearer token", async () => {
    sessionStorage.setItem("network-simulator.access-token", "symbol-access-token");
    const blob = new Blob(["<svg />"], { type: "image/svg+xml" });
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, blob: async () => blob } as Response);
    await expect(api.symbolRaw("custom router.svg")).resolves.toBe(blob);
    expect(fetch).toHaveBeenCalledWith("/v3/symbols/custom%20router.svg/raw", {
      headers: { Authorization: "Bearer symbol-access-token" },
    });
  });

  it("uses the VPCS console WebSocket endpoint", async () => {
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await vpcsConsoleUrl("project-id", "node-id"));

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

  it("uses the same-origin Dynamips IOS console endpoint", async () => {
    vi.mocked(fetch).mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "test-ticket", expires_in: 60 }) } as Response);
    const url = new URL(await dynamipsConsoleUrl("project-id", "node-id"));

    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/v3/compute/projects/project-id/dynamips/nodes/node-id/console/ws");
  });

  it("uses GNS3 link filters, capture, stream, and link update routes", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ link_id: "l1", nodes: [], capturing: true }) } as Response);
    await api.availableLinkFilters("p1", "l1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/links/l1/available_filters", expect.any(Object));

    await api.updateLinkFilters("p1", "l1", { delay: [10, 3] });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/links/l1", expect.objectContaining({ method: "PUT", body: JSON.stringify({ filters: { delay: [10, 3] } }) }));

    await api.startLinkCapture("p1", "l1", { capture_file_name: "edge.pcap", data_link_type: "DLT_EN10MB" });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/links/l1/capture/start", expect.objectContaining({ method: "POST", body: JSON.stringify({ capture_file_name: "edge.pcap", data_link_type: "DLT_EN10MB" }) }));
    expect(api.linkCaptureStreamUrl("p1", "l1")).toBe("/v3/projects/p1/links/l1/capture/stream");

    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: async () => ({ ticket: "capture-ticket", expires_in: 60 }) } as Response);
    await expect(api.accessTicket(api.linkCaptureStreamUrl("p1", "l1"))).resolves.toBe("capture-ticket");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/access/ws-ticket", expect.objectContaining({
      body: JSON.stringify({ path: "/v3/projects/p1/links/l1/capture/stream" }),
    }));

    fetchMock.mockResolvedValueOnce({ ok: true, status: 204 } as Response);
    await api.stopLinkCapture("p1", "l1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/links/l1/capture/stop", expect.objectContaining({ method: "POST" }));
  });

  it("uses project snapshot and symbol controller routes", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] } as Response);
    await api.snapshots("p1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/snapshots", expect.any(Object));
    await api.createSnapshot("p1", "Before changes");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/snapshots", expect.objectContaining({ method: "POST", body: JSON.stringify({ name: "Before changes" }) }));
    await api.listSymbols();
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/symbols", expect.any(Object));
    await api.defaultSymbols();
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/symbols/default_symbols", expect.any(Object));
  });

  it("uses drawing and node isolation routes", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ drawing_id: "d1", node_id: "n1", name: "Router" }) } as Response);
    await api.createDrawing("p1", { x: 1, y: 2, svg: "<svg/>" });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/drawings", expect.objectContaining({ method: "POST" }));
    await api.isolateNode("p1", "n1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/nodes/n1/isolate", expect.objectContaining({ method: "POST" }));
    await api.unisolateNode("p1", "n1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/nodes/n1/unisolate", expect.objectContaining({ method: "POST" }));

    await api.lockProject("p1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/lock", expect.objectContaining({ method: "POST" }));
    await api.unlockProject("p1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/unlock", expect.objectContaining({ method: "POST" }));
  });

  it("uses controller endpoints for project import, export, duplicate, readme, and statistics", async () => {
    const fetchMock = vi.mocked(fetch);
    const file = new File(["archive"], "Core lab.gns3project");
    fetchMock.mockResolvedValueOnce({ ok: true, status: 201, json: async () => ({ project_id: "p2", name: "Imported lab" }) } as Response);
    await api.importProject("p2", "Imported lab", file);
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p2/import?name=Imported+lab", expect.objectContaining({ method: "POST", body: file, headers: { "Content-Type": "application/octet-stream" } }));

    const archive = new Blob(["zip"]);
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, blob: async () => archive } as Response);
    await expect(api.exportProject("p1")).resolves.toBe(archive);
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/export");

    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ project_id: "copy", name: "Core copy", nodes: 2 }) } as Response);
    await api.duplicateProject("p1", "Core copy");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/duplicate", expect.objectContaining({ method: "POST", body: JSON.stringify({ name: "Core copy" }) }));
    await api.projectStatistics("p1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/stats", expect.any(Object));
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, text: async () => "Project notes" } as Response);
    await api.projectReadme("p1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/files/readme.md");
    fetchMock.mockResolvedValueOnce({ ok: false, status: 404, json: async () => ({ message: "Project file does not exist" }) } as Response);
    await expect(api.projectReadme("p1")).resolves.toEqual({ content: "" });
    fetchMock.mockResolvedValueOnce({ ok: true, status: 204 } as Response);
    await api.updateProjectReadme("p1", "Updated notes");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/files/readme.md", expect.objectContaining({
      method: "POST",
      headers: { "Content-Type": "text/plain; charset=utf-8" },
      body: "Updated notes",
    }));
  });

  it("uses settings, compute, template CRUD, and snapshot controller paths", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({}) } as Response);
    await api.settings();
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/settings", expect.any(Object));
    await api.updateSettings({ Server: { port: 3080 } });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/settings", expect.objectContaining({ method: "PUT", body: JSON.stringify({ Server: { port: 3080 } }) }));

    await api.computes();
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/computes", expect.any(Object));
    await api.createTemplate({ name: "My QEMU", template_type: "qemu" });
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/templates", expect.objectContaining({ method: "POST" }));
    await api.duplicateTemplate("template-1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/templates/template-1/duplicate", expect.objectContaining({ method: "POST" }));
    await api.deleteTemplate("template-1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/templates/template-1", expect.objectContaining({ method: "DELETE" }));

    await api.restoreSnapshot("p1", "snap-1");
    expect(fetchMock).toHaveBeenLastCalledWith("/v3/projects/p1/snapshots/snap-1/restore", expect.objectContaining({ method: "POST" }));
  });
});
