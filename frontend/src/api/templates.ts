import type { RuntimeTemplate } from "../api-types";

import { request, jsonBody } from "./client";


export const templatesApi = {
    templates: () => request<RuntimeTemplate[]>("/templates"),
    createTemplate: (template: Record<string, unknown>) => request<RuntimeTemplate>("/templates", jsonBody(template)),
    updateTemplate: (templateId: string, patch: Record<string, unknown>) =>
    request<RuntimeTemplate>(`/templates/${encodeURIComponent(templateId)}`, { method: "PUT", body: JSON.stringify(patch) }),
    duplicateTemplate: (id: string) => request<RuntimeTemplate>(`/templates/${encodeURIComponent(id)}/duplicate`, jsonBody({})),
    deleteTemplate: (id: string) => request<void>(`/templates/${encodeURIComponent(id)}`, { method: "DELETE" })
};
