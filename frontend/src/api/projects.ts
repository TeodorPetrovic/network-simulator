import type { Project, ProjectStatistics } from "../api-types";
import type { Drawing, ProjectSnapshot, TopologySnapshot } from "../api-types";

import { request, jsonBody, fetchWithAuth, handleUnauthorized } from "./client";


export const projectsApi = {
    listProjects: () => request<Project[]>("/projects"),
    createProject: (name: string) => request<Project>("/projects", jsonBody({ name })),
    deleteProject: (id: string) => request<void>(`/projects/${id}`, { method: "DELETE" }),
    updateProject: (id: string, patch: Partial<Project>) => request<Project>(`/projects/${id}`, { method: "PUT", body: JSON.stringify(patch) }),
    projectLocked: (id: string) => request<boolean>(`/projects/${encodeURIComponent(id)}/locked`),
    lockProject: (id: string) => request<void>(`/projects/${encodeURIComponent(id)}/lock`, jsonBody({})),
    unlockProject: (id: string) => request<void>(`/projects/${encodeURIComponent(id)}/unlock`, jsonBody({})),
    duplicateProject: (id: string, name: string) =>
    request<Project>(`/projects/${encodeURIComponent(id)}/duplicate`, jsonBody({ name })),
    projectStatistics: (id: string) => request<ProjectStatistics>(`/projects/${encodeURIComponent(id)}/stats`),
    projectReadme: async (id: string) => {
    const response = await fetchWithAuth(`/v3/projects/${encodeURIComponent(id)}/files/readme.md`);
    handleUnauthorized(response, `/projects/${id}/files/readme.md`);
    if (response.status === 404) return { content: "" };
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project readme is unavailable (${response.status})`);
    }
    return { content: await response.text() };
  },
    updateProjectReadme: async (id: string, content: string) => {
    const response = await fetchWithAuth(`/v3/projects/${encodeURIComponent(id)}/files/readme.md`, {
      method: "POST",
      headers: { "Content-Type": "text/plain; charset=utf-8" },
      body: content,
    });
    handleUnauthorized(response, `/projects/${id}/files/readme.md`);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project readme could not be saved (${response.status})`);
    }
  },
    exportProject: async (id: string) => {
    const response = await fetchWithAuth(`/v3/projects/${encodeURIComponent(id)}/export`);
    handleUnauthorized(response, `/projects/${id}/export`);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project export is unavailable (${response.status})`);
    }
    return response.blob();
  },
    importProject: async (id: string, name: string, file: File) => {
    const params = new URLSearchParams({ name });
    const response = await fetchWithAuth(`/v3/projects/${encodeURIComponent(id)}/import?${params}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    handleUnauthorized(response, `/projects/${id}/import`);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project import is unavailable (${response.status})`);
    }
    return (await response.json()) as Project;
  },
    openProject: (id: string) => request<Project>(`/projects/${id}/open`, jsonBody({})),
    closeProject: (id: string) => request<Project>(`/projects/${id}/close`, jsonBody({})),
    topology: (id: string) => request<TopologySnapshot>(`/projects/${id}/topology`),
    snapshots: (projectId: string) => request<ProjectSnapshot[]>(`/projects/${encodeURIComponent(projectId)}/snapshots`),
    createSnapshot: (projectId: string, name: string) => request<ProjectSnapshot>(`/projects/${encodeURIComponent(projectId)}/snapshots`, jsonBody({ name })),
    restoreSnapshot: (projectId: string, snapshotId: string) => request<Project>(`/projects/${encodeURIComponent(projectId)}/snapshots/${encodeURIComponent(snapshotId)}/restore`, jsonBody({})),
    deleteSnapshot: (projectId: string, snapshotId: string) => request<void>(`/projects/${encodeURIComponent(projectId)}/snapshots/${encodeURIComponent(snapshotId)}`, { method: "DELETE" }),
    createDrawing: (projectId: string, drawing: Omit<Drawing, "drawing_id">) =>
    request<Drawing>(`/projects/${projectId}/drawings`, jsonBody(drawing)),
    updateDrawing: (projectId: string, drawingId: string, patch: Partial<Drawing>) =>
    request<Drawing>(`/projects/${projectId}/drawings/${drawingId}`, { method: "PUT", body: JSON.stringify(patch) }),
    deleteDrawing: (projectId: string, drawingId: string) =>
    request<void>(`/projects/${projectId}/drawings/${drawingId}`, { method: "DELETE" })
};
