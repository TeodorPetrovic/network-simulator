import type { RuntimeTemplate } from "./templates";
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

