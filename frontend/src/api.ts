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

export type VpcsTemplate = {
  template_id: string;
  name: string;
  template_type: "vpcs";
  adapters: 1;
  startup_script?: string;
};

export type DynamipsTemplate = {
  template_id: string;
  name: string;
  template_type: "dynamips";
  platform: "c7200";
  image: string;
  adapters: 1;
  memory: number;
  nvram: number;
  idlepc: string;
};

export type RuntimeTemplate = DockerTemplate | QemuTemplate | VpcsTemplate | DynamipsTemplate;

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
    kvm_accessible: boolean;
    acceleration_error: string | null;
    error: string | null;
  };
  vpcs: {
    available: boolean;
    version: string | null;
    error: string | null;
  };
  dynamips: {
    available: boolean;
    version: string | null;
    binary: string;
    image_directory: string;
    image_count: number;
    error: string | null;
  };
  ubridge: {
    available: boolean;
    binary: string;
    version: string | null;
    error: string | null;
  };
};

export type ResourceMonitor = {
  sampled_at_unix_ms: number | null;
  host: {
    cpu_usage_percent: number | null;
    cpu_count: number | null;
    memory_used_bytes: number | null;
    memory_total_bytes: number | null;
    load_average: number[] | null;
  };
  controller: {
    memory_rss_bytes: number | null;
    thread_count: number | null;
  };
};

export type IshareImage = {
  id: number;
  name: string;
  image_type: string;
  size: string;
  total_size: number;
  file_count: number;
  catalog: string;
};

export type IshareDownloadResult = {
  id: number;
  name: string;
  image_type: string;
  files: string[];
  image_directory: string;
};

export type Gns3ApplianceSearchResult = {
  file_name: string;
  name: string;
  registry_path: string;
  source_url: string;
  download_url: string;
  imported: boolean;
};

export type Gns3ApplianceFile = {
  filename: string;
  version: string | null;
  size: number | null;
  md5sum: string | null;
  download_url: string | null;
};

export type Gns3Appliance = {
  file_name: string;
  appliance_id: string;
  name: string;
  category: string | null;
  description: string | null;
  status: string | null;
  registry_version: number | null;
  template_type: string | null;
  imported: boolean;
  source_url: string;
  required_files: Gns3ApplianceFile[];
};

export type Gns3ApplianceImportResult = {
  appliance: Gns3Appliance;
  matching_local_templates: number;
  message: string;
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
  resourceMonitor: () => request<ResourceMonitor>("/resources/monitor"),
  templates: () => request<RuntimeTemplate[]>("/templates"),
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
  listProjects: () => request<Project[]>("/projects"),
  createProject: (name: string) => request<Project>("/projects", jsonBody({ name })),
  deleteProject: (id: string) => request<void>(`/projects/${id}`, { method: "DELETE" }),
  openProject: (id: string) => request<Project>(`/projects/${id}/open`, jsonBody({})),
  closeProject: (id: string) => request<Project>(`/projects/${id}/close`, jsonBody({})),
  topology: (id: string) => request<TopologySnapshot>(`/projects/${id}/topology`),
  createNode: (projectId: string, node: Omit<Node, "node_id">) =>
    request<Node>(`/projects/${projectId}/nodes`, jsonBody(node)),
  createNodeFromTemplate: (projectId: string, templateId: string, usage: { name: string; x: number; y: number; start_command?: string; startup_script?: string; idlepc?: string; startup_config?: string }) =>
    request<Node>(`/projects/${projectId}/templates/${templateId}`, jsonBody(usage)),
  uploadDynamipsImage: async (file: File) => {
    const response = await fetch(`/v3/dynamips/images/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Image upload failed (${response.status})`);
    }
    return (await response.json()) as { name: string; image: string; template_type: "dynamips"; bytes: number };
  },
  uploadQemuImage: async (file: File) => {
    const response = await fetch(`/v3/qemu/images/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
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

export function qemuConsoleUrl(projectId: string, nodeId: string): string {
  const url = new URL(
    `/v3/compute/projects/${projectId}/qemu/nodes/${nodeId}/console/ws`,
    window.location.href,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function dockerConsoleUrl(projectId: string, nodeId: string): string {
  const url = new URL(
    `/v3/compute/projects/${projectId}/docker/nodes/${nodeId}/console/ws`,
    window.location.href,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function vpcsConsoleUrl(projectId: string, nodeId: string): string {
  const url = new URL(
    `/v3/compute/projects/${projectId}/vpcs/nodes/${nodeId}/console/ws`,
    window.location.href,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function dynamipsConsoleUrl(projectId: string, nodeId: string): string {
  const url = new URL(
    `/v3/compute/projects/${projectId}/dynamips/nodes/${nodeId}/console/ws`,
    window.location.href,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}
