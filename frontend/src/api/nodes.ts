import type { Node } from "../api-types";

import { request, jsonBody, fetchWithAuth, handleUnauthorized } from "./client";


export const nodesApi = {
    /** A file kept with the node (for Docker, `etc/network/interfaces`); null when it was never written. */
    readNodeFile: async (projectId: string, nodeId: string, path: string): Promise<string | null> => {
    const route = `/projects/${encodeURIComponent(projectId)}/nodes/${encodeURIComponent(nodeId)}/files/${path}`;
    const response = await fetchWithAuth(`/v3${route}`);
    handleUnauthorized(response, route);
    if (response.status === 404) return null;
    if (!response.ok) throw new Error(`The file could not be loaded (${response.status})`);
    return response.text();
  },
    writeNodeFile: async (projectId: string, nodeId: string, path: string, content: string) => {
    const route = `/projects/${encodeURIComponent(projectId)}/nodes/${encodeURIComponent(nodeId)}/files/${path}`;
    const response = await fetchWithAuth(`/v3${route}`, { method: "POST", headers: { "Content-Type": "text/plain; charset=utf-8" }, body: content });
    handleUnauthorized(response, route);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `The file could not be saved (${response.status})`);
    }
  },
    createNode: (projectId: string, node: Omit<Node, "node_id">) =>
    request<Node>(`/projects/${projectId}/nodes`, jsonBody(node)),
    createNodeFromTemplate: (projectId: string, templateId: string, usage: { name: string; x: number; y: number; start_command?: string; startup_script?: string; idlepc?: string; mmap?: boolean; sparsemem?: boolean; auto_delete_disks?: boolean; ghost_ios?: boolean; console_auto_start?: boolean; startup_config?: string }) =>
    request<Node>(`/projects/${projectId}/templates/${templateId}`, jsonBody(usage)),
    uploadDynamipsImage: async (file: File) => {
    const response = await fetchWithAuth(`/v3/dynamips/images/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    handleUnauthorized(response, "/dynamips/images");
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Image upload failed (${response.status})`);
    }
    return (await response.json()) as { name: string; image: string; template_type: "dynamips"; bytes: number };
  },
    uploadQemuImage: async (file: File) => {
    const response = await fetchWithAuth(`/v3/qemu/images/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    handleUnauthorized(response, "/qemu/images");
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `QEMU image upload failed (${response.status})`);
    }
    return (await response.json()) as { name: string; image: string; template_type: "qemu"; bytes: number };
  },
    updateNode: (projectId: string, nodeId: string, patch: Partial<Node>) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}`, {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
    configureNode: (projectId: string, nodeId: string, patch: Record<string, unknown>) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/config`, {
      method: "PUT",
      body: JSON.stringify(patch),
    }),
    deleteNode: (projectId: string, nodeId: string) =>
    request<void>(`/projects/${projectId}/nodes/${nodeId}`, { method: "DELETE" }),
    startNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/start`, jsonBody({})),
    stopNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/stop`, jsonBody({})),
    suspendNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/suspend`, jsonBody({})),
    reloadNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/reload`, jsonBody({})),
    isolateNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/isolate`, jsonBody({})),
    unisolateNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/unisolate`, jsonBody({}))
};
