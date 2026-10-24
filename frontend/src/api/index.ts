import { authApi } from "./auth";
import { projectsApi } from "./projects";
import { nodesApi } from "./nodes";
import { linksApi } from "./links";
import { templatesApi } from "./templates";
import { computesApi } from "./computes";
import { appliancesApi } from "./appliances";
import { systemApi } from "./system";
import { getAuthToken } from "./client";

export const api = { ...authApi, ...systemApi, ...templatesApi, ...appliancesApi, ...projectsApi, ...computesApi, ...nodesApi, ...linksApi };
export type * from "../api-types";
export { getAuthToken } from "./client";

export function projectNotificationsUrl(projectId: string, after: number): Promise<string> {
  return withSocketTicket(`/v3/projects/${projectId}/notifications/ws`, { after: String(after) });
}
async function withSocketTicket(path: string, query: Record<string, string> = {}): Promise<string> {
  const url = new URL(path, window.location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  for (const [key, value] of Object.entries(query)) url.searchParams.set(key, value);
  if (getAuthToken()) url.searchParams.set("ticket", await api.accessTicket(path));
  return url.toString();
}
export function qemuConsoleUrl(projectId: string, nodeId: string): Promise<string> { return withSocketTicket(`/v3/compute/projects/${projectId}/qemu/nodes/${nodeId}/console/ws`); }
export function dockerConsoleUrl(projectId: string, nodeId: string): Promise<string> { return withSocketTicket(`/v3/compute/projects/${projectId}/docker/nodes/${nodeId}/console/ws`); }
export function dockerAuxConsoleUrl(projectId: string, nodeId: string): Promise<string> { return withSocketTicket(`/v3/compute/projects/${projectId}/docker/nodes/${nodeId}/aux_console/ws`); }
export function dockerHttpConsoleUrl(protocol: "http" | "https", host: string, port: number, path: string): string | null {
  if (!host.trim() || !Number.isInteger(port) || port < 1 || port > 65_535) return null;
  try { const url = new URL(`${protocol}://${host}`); url.port = String(port); url.pathname = path.startsWith("/") ? path : `/${path}`; return url.toString(); } catch { return null; }
}
export function symbolRawUrl(symbolId: string): string { return `/v3/symbols/${encodeURIComponent(symbolId)}/raw`; }
export function vpcsConsoleUrl(projectId: string, nodeId: string): Promise<string> { return withSocketTicket(`/v3/compute/projects/${projectId}/vpcs/nodes/${nodeId}/console/ws`); }
export function dynamipsConsoleUrl(projectId: string, nodeId: string): Promise<string> { return withSocketTicket(`/v3/compute/projects/${projectId}/dynamips/nodes/${nodeId}/console/ws`); }
