export type Project = {
  project_id: string;
  name: string;
  status?: string;
  auto_open?: boolean;
  auto_start?: boolean;
  auto_close?: boolean;
  show_interface_labels?: boolean;
  scene_width?: number;
  scene_height?: number;
  grid_size?: number;
  drawing_grid_size?: number;
  variables?: Array<{ name: string; value: string }>;
  [key: string]: unknown;
};

export type Compute = {
  compute_id: string;
  name: string;
  host?: string;
  port?: number;
  protocol?: string;
  connected?: boolean;
  [key: string]: unknown;
};

export type ProjectStatistics = {
  nodes?: number;
  links?: number;
  drawings?: number;
  [key: string]: unknown;
};

export type ServerSettings = Record<string, unknown>;

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
  label?: LinkEndpointLabel;
};

export type LinkEndpointLabel = {
  text: string;
  style?: string | null;
  x?: number | null;
  y?: number | null;
  rotation?: number;
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
  z_index?: number;
  svg?: string;
  rotation?: number;
  [key: string]: unknown;
};

export type ProjectSnapshot = {
  snapshot_id: string;
  name: string;
  created_at?: number;
  [key: string]: unknown;
};

export type NetworkSymbol = {
  symbol_id?: string;
  name?: string;
  filename?: string;
  builtin?: boolean;
  raw?: string;
  path?: string;
  [key: string]: unknown;
};

export type LinkFilterDescription = {
  name: string;
  description: string;
  type?: string;
  parameters?: Array<{ name: string; type: string; minimum?: number; maximum?: number; unit?: string }>;
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
  environment?: string;
  extra_hosts?: string;
  extra_volumes?: string[];
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
  platform?: string;
  adapter_type?: string;
  hda_disk_interface?: string;
  boot_priority?: string;
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
  platform: "c7200" | "c3600";
  chassis?: string | null;
  slot0?: string;
  image: string;
  adapters: 1;
  memory: number;
  nvram: number;
  idlepc: string;
};

export type EthernetSwitchPort = {
  name: string;
  port_number: number;
  type: "access" | "dot1q" | "qinq";
  vlan: number;
  ethertype?: "0x8100" | "0x88a8" | "0x9100" | "0x9200";
};

export type EthernetHubPort = { name: string; port_number: number };

export type CloudPort =
  | { name: string; port_number: number; type: "ethernet" | "tap"; interface: string }
  | { name: string; port_number: number; type: "udp"; lport: number; rhost: string; rport: number };

export type BuiltinTemplate = {
  template_id: string;
  name: string;
  template_type: "ethernet_switch" | "ethernet_hub" | "cloud" | "nat";
  builtin: true;
  ports_mapping: Array<EthernetSwitchPort | EthernetHubPort | CloudPort>;
  remote_console_host?: string;
  remote_console_port?: number;
  remote_console_type?: "none" | "telnet" | "ssh" | "vnc" | "spice" | "http" | "https";
  remote_console_http_path?: string;
  nat_interface?: string;
  category?: string;
  symbol?: string;
};

export type RuntimeTemplate = DockerTemplate | QemuTemplate | VpcsTemplate | DynamipsTemplate | BuiltinTemplate;

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
  files?: string[];
  catalog: string;
};

export type IshareDownloadResult = {
  id: number;
  name: string;
  image_type: string;
  files: string[];
  source_files?: string[];
  image_directory: string;
};

export type Gns3ApplianceSearchResult = {
  file_name: string;
  name: string;
  category?: string | null;
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
  origin?: "gns3" | "custom";
  category: string | null;
  description: string | null;
  status: string | null;
  registry_version: number | null;
  template_type: string | null;
  imported: boolean;
  source_url: string;
  required_files: Gns3ApplianceFile[];
  versions?: ApplianceVersion[];
};

export type ApplianceImageRequirement = {
  key: string;
  filename: string;
  assignable: boolean;
  filesize: number | null;
  md5sum: string | null;
  assigned_image_id: string | null;
  assigned_image_name: string | null;
  suggested_image_id: string | null;
  suggested_image_name: string | null;
  image_path: string | null;
  actual_size: number | null;
  actual_md5sum: string | null;
  actual_sha1sum: string | null;
  status: "missing" | "verified" | "size-verified" | "available-unverified" | "mismatch" | "override" | "assigned-image-missing" | string;
  allow_checksum_mismatch: boolean;
};

export type ApplianceVersion = {
  name: string;
  settings: string | null;
  template_type: string | null;
  runtime_supported: boolean;
  docker_image?: string | null;
  docker_image_id?: string | null;
  docker_image_available?: boolean;
  images: ApplianceImageRequirement[];
};

export type LocalApplianceImage = {
  image_id: string;
  name: string;
  template_type: "qemu" | "dynamips";
  path: string;
  size: number;
  md5sum: string | null;
  sha1sum: string | null;
};

export type ApplianceManagerState = {
  appliances: Gns3Appliance[];
  local_images: LocalApplianceImage[];
  custom_image_options: RuntimeTemplate[];
};

export type CreateCustomApplianceRequest = {
  name: string;
  source_template_id: string;
  runtime_settings?: Record<string, unknown>;
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
  updateTemplate: (templateId: string, patch: Record<string, unknown>) =>
    request<RuntimeTemplate>(`/templates/${encodeURIComponent(templateId)}`, { method: "PUT", body: JSON.stringify(patch) }),
  pullDockerImage: (image: string) => request<{ image_id: string; template: RuntimeTemplate }>("/docker/images/pull", jsonBody({ image })),
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
    const response = await fetch(`/v3/marketplace/gns3/import/${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Appliance import failed (${response.status})`);
    }
    return (await response.json()) as Gns3Appliance;
  },
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
    const response = await fetch(`/v3/projects/${encodeURIComponent(id)}/files/readme.md`);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project readme is unavailable (${response.status})`);
    }
    return { content: await response.text() };
  },
  updateProjectReadme: async (id: string, content: string) => {
    const response = await fetch(`/v3/projects/${encodeURIComponent(id)}/files/readme.md`, {
      method: "POST",
      headers: { "Content-Type": "text/plain; charset=utf-8" },
      body: content,
    });
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project readme could not be saved (${response.status})`);
    }
  },
  exportProject: async (id: string) => {
    const response = await fetch(`/v3/projects/${encodeURIComponent(id)}/export`);
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project export is unavailable (${response.status})`);
    }
    return response.blob();
  },
  importProject: async (id: string, name: string, file: File) => {
    const params = new URLSearchParams({ name });
    const response = await fetch(`/v3/projects/${encodeURIComponent(id)}/import?${params}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    if (!response.ok) {
      const body = (await response.json().catch(() => null)) as { message?: string } | null;
      throw new Error(body?.message ?? `Project import is unavailable (${response.status})`);
    }
    return (await response.json()) as Project;
  },
  settings: () => request<ServerSettings>("/settings"),
  updateSettings: (settings: ServerSettings) =>
    request<ServerSettings>("/settings", { method: "PUT", body: JSON.stringify(settings) }),
  computes: () => request<Compute[]>("/computes"),
  createTemplate: (template: Record<string, unknown>) => request<RuntimeTemplate>("/templates", jsonBody(template)),
  duplicateTemplate: (id: string) => request<RuntimeTemplate>(`/templates/${encodeURIComponent(id)}/duplicate`, jsonBody({})),
  deleteTemplate: (id: string) => request<void>(`/templates/${encodeURIComponent(id)}`, { method: "DELETE" }),
  openProject: (id: string) => request<Project>(`/projects/${id}/open`, jsonBody({})),
  closeProject: (id: string) => request<Project>(`/projects/${id}/close`, jsonBody({})),
  topology: (id: string) => request<TopologySnapshot>(`/projects/${id}/topology`),
  snapshots: (projectId: string) => request<ProjectSnapshot[]>(`/projects/${encodeURIComponent(projectId)}/snapshots`),
  createSnapshot: (projectId: string, name: string) => request<ProjectSnapshot>(`/projects/${encodeURIComponent(projectId)}/snapshots`, jsonBody({ name })),
  restoreSnapshot: (projectId: string, snapshotId: string) => request<Project>(`/projects/${encodeURIComponent(projectId)}/snapshots/${encodeURIComponent(snapshotId)}/restore`, jsonBody({})),
  deleteSnapshot: (projectId: string, snapshotId: string) => request<void>(`/projects/${encodeURIComponent(projectId)}/snapshots/${encodeURIComponent(snapshotId)}`, { method: "DELETE" }),
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
  configureNode: (projectId: string, nodeId: string, patch: Record<string, unknown>) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/config`, {
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
    `/v3/projects/${projectId}/links/${linkId}/capture/stream` as const,
  listSymbols: () => request<NetworkSymbol[]>("/symbols"),
  defaultSymbols: () => request<{ default_symbol: string; symbols: Record<string, string> }>("/symbols/default_symbols"),
  createDrawing: (projectId: string, drawing: Omit<Drawing, "drawing_id">) =>
    request<Drawing>(`/projects/${projectId}/drawings`, jsonBody(drawing)),
  updateDrawing: (projectId: string, drawingId: string, patch: Partial<Drawing>) =>
    request<Drawing>(`/projects/${projectId}/drawings/${drawingId}`, { method: "PUT", body: JSON.stringify(patch) }),
  deleteDrawing: (projectId: string, drawingId: string) =>
    request<void>(`/projects/${projectId}/drawings/${drawingId}`, { method: "DELETE" }),
  isolateNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/isolate`, jsonBody({})),
  unisolateNode: (projectId: string, nodeId: string) =>
    request<Node>(`/projects/${projectId}/nodes/${nodeId}/unisolate`, jsonBody({})),
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

export function dockerAuxConsoleUrl(projectId: string, nodeId: string): string {
  const url = new URL(
    `/v3/compute/projects/${projectId}/docker/nodes/${nodeId}/aux_console/ws`,
    window.location.href,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function dockerHttpConsoleUrl(protocol: "http" | "https", host: string, port: number, path: string): string | null {
  if (!host.trim() || !Number.isInteger(port) || port < 1 || port > 65_535) return null;
  try {
    const url = new URL(`${protocol}://${host}`);
    url.port = String(port);
    url.pathname = path.startsWith("/") ? path : `/${path}`;
    return url.toString();
  } catch {
    return null;
  }
}

export function symbolRawUrl(symbolId: string): string {
  return `/v3/symbols/${encodeURIComponent(symbolId)}/raw`;
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
