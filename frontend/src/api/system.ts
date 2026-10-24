import type { ServerSettings } from "../api-types";
import type { NetworkSymbol, RuntimeTemplate, RuntimeInfo, ResourceMonitor } from "../api-types";

import { request, jsonBody, fetchWithAuth, handleUnauthorized } from "./client";


export const systemApi = {
    runtimes: () => request<RuntimeInfo>("/runtimes"),
    resourceMonitor: () => request<ResourceMonitor>("/resources/monitor"),
    settings: () => request<ServerSettings>("/settings"),
    updateSettings: (settings: ServerSettings) =>
    request<ServerSettings>("/settings", { method: "PUT", body: JSON.stringify(settings) }),
    pullDockerImage: (image: string) => request<{ image_id: string; template: RuntimeTemplate }>("/docker/images/pull", jsonBody({ image })),
    symbolRaw: async (symbolId: string) => {
    const path = `/symbols/${encodeURIComponent(symbolId)}/raw`;
    const response = await fetchWithAuth(`/v3${path}`);
    handleUnauthorized(response, path);
    if (!response.ok) {
      const body = await response.json().catch(() => null) as { message?: string } | null;
      throw new Error(body?.message ?? `Request failed (${response.status})`);
    }
    return response.blob();
  },
    listSymbols: () => request<NetworkSymbol[]>("/symbols"),
    defaultSymbols: () => request<{ default_symbol: string; symbols: Record<string, string> }>("/symbols/default_symbols")
};
