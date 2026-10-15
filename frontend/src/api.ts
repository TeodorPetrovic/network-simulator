export type Project = {
  project_id: string;
  name: string;
  status?: string;
  [key: string]: unknown;
};

export type Node = {
  node_id: string;
  name: string;
  node_type?: string;
  status?: string;
  x?: number;
  y?: number;
  [key: string]: unknown;
};

export type LinkEndpoint = {
  node_id: string;
  adapter_number: number;
  port_number: number;
};

export type Link = {
  link_id: string;
  nodes: LinkEndpoint[];
  link_type?: string;
  [key: string]: unknown;
};

export type Drawing = {
  drawing_id: string;
  x?: number;
  y?: number;
  [key: string]: unknown;
};

export type TopologySnapshot = {
  project: Project;
  nodes: Node[];
  links: Link[];
  drawings: Drawing[];
  event_cursor: number;
};

export type ProjectEvent = {
  project_id: string;
  sequence: number;
  action: string;
  event: Record<string, unknown>;
};

export type DockerTemplate = {
  template_id: string;
  name: string;
  template_type: "docker";
  image: string;
  image_digest: string;
  start_command?: string;
  adapters: number;
  memory: number;
  cpus: number;
};

export type QemuTemplate = {
  template_id: string;
  name: string;
  template_type: "qemu";
  hda_disk_image: string;
  adapters: number;
  memory: number;
  cpus: number;
};

export type RuntimeTemplate = DockerTemplate | QemuTemplate;

export type RuntimeInfo = {
  docker: {
    available: boolean;
    version: string | null;
    error: string | null;
  };
  qemu: {
    available: boolean;
    version: string | null;
    qemu_img_available: boolean;
    image_directory: string;
    image_count: number;
    acceleration: string;
    error: string | null;
  };
};

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`/v3${path}`, {
    ...init,
    headers: {
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { message?: string } | null;
    throw new Error(body?.message ?? `Request failed (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

const jsonBody = (value: unknown): Pick<RequestInit, "body" | "method"> => ({
  method: "POST",
  body: JSON.stringify(value),
});

export const api = {
  runtimes: () => request<RuntimeInfo>("/runtimes"),
  templates: () => request<RuntimeTemplate[]>("/templates"),
  listProjects: () => request<Project[]>("/projects"),
  createProject: (name: string) => request<Project>("/projects", jsonBody({ name })),
  deleteProject: (id: string) => request<void>(`/projects/${id}`, { method: "DELETE" }),
  openProject: (id: string) => request<Project>(`/projects/${id}/open`, jsonBody({})),
  closeProject: (id: string) => request<Project>(`/projects/${id}/close`, jsonBody({})),
  topology: (id: string) => request<TopologySnapshot>(`/projects/${id}/topology`),
  createNode: (projectId: string, node: Omit<Node, "node_id">) =>
    request<Node>(`/projects/${projectId}/nodes`, jsonBody(node)),
  createNodeFromTemplate: (projectId: string, templateId: string, usage: { name: string; x: number; y: number; start_command?: string }) =>
    request<Node>(`/projects/${projectId}/templates/${templateId}`, jsonBody(usage)),
  updateNode: (projectId: string, nodeId: string, patch: Partial<Node>) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}`, {
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
  createLink: (projectId: string, nodes: LinkEndpoint[]) =>
    request<Link>(`/projects/${projectId}/links`, jsonBody({ nodes, link_type: "ethernet" })),
  deleteLink: (projectId: string, linkId: string) =>
    request<void>(`/projects/${projectId}/links/${linkId}`, { method: "DELETE" }),
};

export function projectNotificationsUrl(projectId: string, after: number): string {
  const url = new URL(`/v3/projects/${projectId}/notifications/ws`, window.location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.searchParams.set("after", String(after));
  return url.toString();
}
