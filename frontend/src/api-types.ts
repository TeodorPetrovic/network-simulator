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

