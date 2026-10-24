import type { AccessUser, AccessGroup, AccessRole, AccessPrivilege, AccessControlEntry, ResourcePool, UserApiKey } from "../api-types";

import { request, jsonBody, saveAuthToken } from "./client";


export const authApi = {
    authStatus: () => request<{ auth_enabled: boolean }>("/access/settings/status"),
    accessTicket: async (path: string) => {
    const result = await request<{ ticket: string; expires_in: number }>("/access/ws-ticket", jsonBody({ path }));
    return result.ticket;
  },
    login: async (username: string, password: string) => {
    const response = await fetch("/v3/access/users/login", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ username, password }),
    });
    const body = await response.json().catch(() => null) as { access_token?: string; must_change_password?: boolean; message?: string } | null;
    if (!response.ok || !body?.access_token) throw new Error(body?.message ?? `Login failed (${response.status})`);
    saveAuthToken(body.access_token);
    return body;
  },
    logout: async () => {
    try { await request<void>("/access/users/logout", { method: "POST" }); }
    finally { saveAuthToken(null); }
  },
    me: () => request<AccessUser>("/access/users/me"),
    updateMe: async (patch: { password?: string; email?: string; full_name?: string }) => {
    const user = await request<AccessUser>("/access/users/me", { method: "PUT", body: JSON.stringify(patch) });
    if (patch.password) saveAuthToken(null);
    return user;
  },
    users: () => request<AccessUser[]>("/access/users"),
    createUser: (user: { username: string; password: string; email?: string; full_name?: string; is_active?: boolean }) => request<AccessUser>("/access/users", jsonBody(user)),
    updateUser: (userId: string, patch: Partial<AccessUser> & { password?: string }) => request<AccessUser>(`/access/users/${encodeURIComponent(userId)}`, { method: "PUT", body: JSON.stringify(patch) }),
    deleteUser: (userId: string) => request<void>(`/access/users/${encodeURIComponent(userId)}`, { method: "DELETE" }),
    groups: () => request<AccessGroup[]>("/access/groups"),
    createGroup: (name: string) => request<AccessGroup>("/access/groups", jsonBody({ name })),
    updateGroup: (groupId: string, name: string) => request<AccessGroup>(`/access/groups/${encodeURIComponent(groupId)}`, { method: "PUT", body: JSON.stringify({ name }) }),
    deleteGroup: (groupId: string) => request<void>(`/access/groups/${encodeURIComponent(groupId)}`, { method: "DELETE" }),
    groupMembers: (groupId: string) => request<AccessUser[]>(`/access/groups/${encodeURIComponent(groupId)}/members`),
    addGroupMember: (groupId: string, userId: string) => request<void>(`/access/groups/${encodeURIComponent(groupId)}/members/${encodeURIComponent(userId)}`, { method: "PUT" }),
    removeGroupMember: (groupId: string, userId: string) => request<void>(`/access/groups/${encodeURIComponent(groupId)}/members/${encodeURIComponent(userId)}`, { method: "DELETE" }),
    roles: () => request<AccessRole[]>("/access/roles"),
    createRole: (name: string, description?: string) => request<AccessRole>("/access/roles", jsonBody({ name, description })),
    updateRole: (roleId: string, patch: { name?: string; description?: string }) => request<AccessRole>(`/access/roles/${encodeURIComponent(roleId)}`, { method: "PUT", body: JSON.stringify(patch) }),
    deleteRole: (roleId: string) => request<void>(`/access/roles/${encodeURIComponent(roleId)}`, { method: "DELETE" }),
    privileges: () => request<AccessPrivilege[]>("/access/privileges"),
    addRolePrivilege: (roleId: string, privilegeId: string) => request<void>(`/access/roles/${encodeURIComponent(roleId)}/privileges/${encodeURIComponent(privilegeId)}`, { method: "PUT" }),
    removeRolePrivilege: (roleId: string, privilegeId: string) => request<void>(`/access/roles/${encodeURIComponent(roleId)}/privileges/${encodeURIComponent(privilegeId)}`, { method: "DELETE" }),
    acl: () => request<AccessControlEntry[]>("/access/acl"),
    aclEndpoints: () => request<Array<{ endpoint: string; name: string; endpoint_type: string }>>("/access/acl/endpoints"),
    createAcl: (entry: Omit<AccessControlEntry, "ace_id">) => request<AccessControlEntry>("/access/acl", jsonBody(entry)),
    updateAcl: (aceId: string, patch: Partial<AccessControlEntry>) => request<AccessControlEntry>(`/access/acl/${encodeURIComponent(aceId)}`, { method: "PUT", body: JSON.stringify(patch) }),
    deleteAcl: (aceId: string) => request<void>(`/access/acl/${encodeURIComponent(aceId)}`, { method: "DELETE" }),
    pools: () => request<ResourcePool[]>("/pools"),
    createPool: (name: string) => request<ResourcePool>("/pools", jsonBody({ name })),
    updatePool: (poolId: string, name: string) => request<ResourcePool>(`/pools/${encodeURIComponent(poolId)}`, { method: "PUT", body: JSON.stringify({ name }) }),
    deletePool: (poolId: string) => request<void>(`/pools/${encodeURIComponent(poolId)}`, { method: "DELETE" }),
    poolResources: (poolId: string) => request<Array<{ resource_id: string; resource_type: string; name?: string | null }>>(`/pools/${encodeURIComponent(poolId)}/resources`),
    addPoolResource: (poolId: string, resourceId: string) => request<void>(`/pools/${encodeURIComponent(poolId)}/resources/${encodeURIComponent(resourceId)}`, { method: "PUT" }),
    removePoolResource: (poolId: string, resourceId: string) => request<void>(`/pools/${encodeURIComponent(poolId)}/resources/${encodeURIComponent(resourceId)}`, { method: "DELETE" }),
    apiKeys: () => request<UserApiKey[]>("/access/api-keys"),
    createApiKey: (name: string) => request<UserApiKey & { api_key: string }>("/access/api-keys", jsonBody({ name })),
    revokeApiKey: (keyId: string) => request<{ message: string }>(`/access/api-keys/${encodeURIComponent(keyId)}/revoke`, jsonBody({})),
    restoreApiKey: (keyId: string) => request<{ message: string }>(`/access/api-keys/${encodeURIComponent(keyId)}/restore`, jsonBody({})),
    deleteApiKey: (keyId: string) => request<void>(`/access/api-keys/${encodeURIComponent(keyId)}`, { method: "DELETE" }),
    authSettings: () => request<{ auth_enabled: boolean }>("/access/settings"),
    updateAuthSettings: (auth_enabled: boolean) => request<{ auth_enabled: boolean }>("/access/settings", { method: "PUT", body: JSON.stringify({ auth_enabled }) })
};
