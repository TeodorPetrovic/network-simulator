import { beforeEach, describe, expect, it, vi } from "vitest";
import { api, projectNotificationsUrl } from "./api";

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
});
