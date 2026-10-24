import type { Compute } from "../api-types";

import { request } from "./client";


export const computesApi = {
    computes: () => request<Compute[]>("/computes")
};
