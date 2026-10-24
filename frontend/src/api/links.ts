import type { LinkEndpoint, Link, LinkFilterDescription } from "../api-types";

import { request, jsonBody } from "./client";


export const linksApi = {
    createLink: (projectId: string, nodes: LinkEndpoint[]) =>
    request<Link>(`/projects/${projectId}/links`, jsonBody({ nodes, link_type: "ethernet" })),
    deleteLink: (projectId: string, linkId: string) =>
    request<void>(`/projects/${projectId}/links/${linkId}`, { method: "DELETE" }),
    updateLink: (projectId: string, linkId: string, patch: Partial<Link>) =>
    request<Link>(`/projects/${projectId}/links/${linkId}`, { method: "PUT", body: JSON.stringify(patch) }),
    linkFilters: (link: Link) => (link.filters ?? {}) as Record<string, unknown>,
    updateLinkFilters: (projectId: string, linkId: string, filters: Record<string, unknown>) =>
    request<Link>(`/projects/${projectId}/links/${linkId}`, { method: "PUT", body: JSON.stringify({ filters }) }),
    availableLinkFilters: (projectId: string, linkId: string) =>
    request<LinkFilterDescription[]>(`/projects/${projectId}/links/${linkId}/available_filters`),
    startLinkCapture: (projectId: string, linkId: string, settings: { capture_file_name: string; data_link_type: "DLT_EN10MB" }) =>
    request<Link>(`/projects/${projectId}/links/${linkId}/capture/start`, jsonBody(settings)),
    stopLinkCapture: (projectId: string, linkId: string) =>
    request<void>(`/projects/${projectId}/links/${linkId}/capture/stop`, jsonBody({})),
    linkCaptureStreamUrl: (projectId: string, linkId: string) =>
    `/v3/projects/${projectId}/links/${linkId}/capture/stream` as const
};
