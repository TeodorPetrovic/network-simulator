import type { IshareImage, IshareDownloadResult, Gns3ApplianceSearchResult, Gns3Appliance, ApplianceManagerState, CreateCustomApplianceRequest, Gns3ApplianceImportResult } from "../api-types";

import { request, jsonBody, fetchWithAuth, handleUnauthorized } from "./client";


export const appliancesApi = {
    searchIshareImages: async (query: string, kind: string) => {
    const params = new URLSearchParams({ q: query, kind });
    const response = await request<{ source: string; results: IshareImage[] }>(`/marketplace/ishare2/images?${params}`);
    return response.results;
  },
    downloadIshareImage: (image: IshareImage) =>
    request<IshareDownloadResult>(`/marketplace/ishare2/images/${encodeURIComponent(image.image_type)}/${image.id}/download`, jsonBody({})),
    searchGns3Appliances: async (query: string) => {
    const params = new URLSearchParams({ q: query });
    const response = await request<{ source: string; results: Gns3ApplianceSearchResult[] }>(`/marketplace/gns3/appliances?${params}`);
    return response.results;
  },
    gns3Appliance: (fileName: string) =>
    request<Gns3Appliance>(`/marketplace/gns3/appliances/${encodeURIComponent(fileName)}`),
    importGns3Appliance: (fileName: string) =>
    request<Gns3ApplianceImportResult>(`/marketplace/gns3/appliances/${encodeURIComponent(fileName)}/import`, jsonBody({})),
    importedGns3Appliances: () => request<Gns3Appliance[]>("/marketplace/gns3/imported"),
    applianceManager: () => request<ApplianceManagerState>("/marketplace/gns3/manager"),
    createCustomAppliance: (body: CreateCustomApplianceRequest) =>
    request<Gns3Appliance>("/marketplace/gns3/custom", jsonBody(body)),
    setApplianceImageBinding: (fileName: string, binding: { version: string; image_key: string; image_id: string; allow_checksum_mismatch: boolean }) =>
    request<{ appliance: Gns3Appliance }>(`/marketplace/gns3/appliances/${encodeURIComponent(fileName)}/bindings`, { ...jsonBody(binding), method: "PUT" }),
    clearApplianceImageBinding: (fileName: string, binding: { version: string; image_key: string }) =>
    request<{ appliance: Gns3Appliance }>(`/marketplace/gns3/appliances/${encodeURIComponent(fileName)}/bindings`, { ...jsonBody(binding), method: "DELETE" }),
    importGns3ApplianceFile: async (file: File) => {
    const response = await fetchWithAuth(`/v3/marketplace/gns3/import/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    handleUnauthorized(response, "/marketplace/gns3/import");
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Appliance import failed (${response.status})`);
    }
    return (await response.json()) as Gns3Appliance;
  }
};
