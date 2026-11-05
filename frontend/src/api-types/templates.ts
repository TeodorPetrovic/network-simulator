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
  image_missing?: boolean;
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
  platform: "c1700" | "c2600" | "c2691" | "c3600" | "c3725" | "c3745" | "c7200";
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
  image_missing?: boolean;
};

export type IouTemplate = {
  template_id: string;
  name: string;
  template_type: "iou";
  path?: string;
  image?: string;
  ram?: number;
  nvram?: number;
  ethernet_adapters?: number;
  serial_adapters?: number;
  console_type?: string;
  startup_config?: string;
  private_config?: string;
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

export type RuntimeTemplate = DockerTemplate | QemuTemplate | VpcsTemplate | DynamipsTemplate | IouTemplate | BuiltinTemplate;

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

