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

export type AccessUser = { user_id: string; username: string; email?: string | null; full_name?: string | null; is_active: boolean; is_superadmin: boolean; must_change_password?: boolean };
export type AccessGroup = { user_group_id: string; name: string; is_builtin: boolean };
export type AccessPrivilege = { privilege_id: string; name: string; description?: string | null };
export type AccessRole = { role_id: string; name: string; description?: string | null; is_builtin: boolean; privileges: AccessPrivilege[] };
export type AccessControlEntry = { ace_id: string; ace_type: "user" | "group"; path: string; propagate: boolean; allowed: boolean; user_id?: string | null; group_id?: string | null; role_id: string };
export type ResourcePool = { resource_pool_id: string; name: string };
export type UserApiKey = { api_key_id: string; name: string; key_prefix: string; revoked: boolean; created_at?: string; last_used_at?: string | null };


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
  mmap?: boolean;
  sparsemem?: boolean;
  auto_delete_disks?: boolean;
  ghost_ios?: boolean;
  console_auto_start?: boolean;
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

