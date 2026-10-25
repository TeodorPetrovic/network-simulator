import type { Compute } from "../api-types";

import { request } from "./client";


export const computesApi = {
    computes: () => request<Compute[]>("/computes"),
    createCompute: (compute: Omit<Compute, "connected"> & { user?: string; password?: string }) => request<Compute>("/computes", { method: "POST", body: JSON.stringify(compute) }),
    updateCompute: (id: string, compute: Omit<Compute, "compute_id" | "connected"> & { user?: string; password?: string }) => request<Compute>(`/computes/${encodeURIComponent(id)}`, { method: "PUT", body: JSON.stringify(compute) }),
    deleteCompute: (id: string) => request<void>(`/computes/${encodeURIComponent(id)}`, { method: "DELETE" })
};
