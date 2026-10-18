import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { Link } from "@tanstack/react-router";
import type { AccessControlEntry, AccessGroup, AccessPrivilege, AccessRole, AccessUser, Project, ResourcePool } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";

export type AccessSection = "users" | "groups" | "roles" | "acl" | "pools";
const sections: Array<{ id: AccessSection; label: string }> = [
  { id: "users", label: "Users" }, { id: "groups", label: "Groups" }, { id: "roles", label: "Roles" }, { id: "acl", label: "Access control" }, { id: "pools", label: "Resource pools" },
];
const message = (reason: unknown) => reason instanceof Error ? reason.message : "The request could not be completed.";

export function AccessManagementPage({ section }: { section: AccessSection }) {
  const [users, setUsers] = useState<AccessUser[]>([]);
  const [groups, setGroups] = useState<AccessGroup[]>([]);
  const [roles, setRoles] = useState<AccessRole[]>([]);
  const [privileges, setPrivileges] = useState<AccessPrivilege[]>([]);
  const [entries, setEntries] = useState<AccessControlEntry[]>([]);
  const [endpoints, setEndpoints] = useState<Array<{ endpoint: string; name: string }>>([]);
  const [pools, setPools] = useState<ResourcePool[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [members, setMembers] = useState<Record<string, AccessUser[]>>({});
  const [poolResources, setPoolResources] = useState<Record<string, Array<{ resource_id: string; name?: string | null }>>>({});
  const [name, setName] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [fullName, setFullName] = useState("");
  const [email, setEmail] = useState("");
  const [selectedRole, setSelectedRole] = useState("");
  const [aclType, setAclType] = useState<"user" | "group">("user");
  const [aclPrincipal, setAclPrincipal] = useState("");
  const [aclPath, setAclPath] = useState("/projects");
  const [aclAllowed, setAclAllowed] = useState(true);
  const [aclPropagate, setAclPropagate] = useState(true);
  const [selectedPool, setSelectedPool] = useState("");
  const [selectedProject, setSelectedProject] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true); setError("");
    try {
      if (section === "users") setUsers(await api.users());
      if (section === "groups") {
        const [nextGroups, nextUsers] = await Promise.all([api.groups(), api.users()]);
        setGroups(nextGroups); setUsers(nextUsers);
        const loaded = await Promise.all(nextGroups.map(async (group) => [group.user_group_id, await api.groupMembers(group.user_group_id)] as const));
        setMembers(Object.fromEntries(loaded));
      }
      if (section === "roles") {
        const [nextRoles, nextPrivileges] = await Promise.all([api.roles(), api.privileges()]);
        setRoles(nextRoles); setPrivileges(nextPrivileges);
      }
      if (section === "acl") {
        const [nextEntries, nextUsers, nextGroups, nextRoles, nextEndpoints, nextPools] = await Promise.all([api.acl(), api.users(), api.groups(), api.roles(), api.aclEndpoints(), api.pools()]);
        setEntries(nextEntries); setUsers(nextUsers); setGroups(nextGroups); setRoles(nextRoles); setEndpoints(nextEndpoints); setPools(nextPools);
      }
      if (section === "pools") {
        const [nextPools, nextProjects] = await Promise.all([api.pools(), api.listProjects()]);
        setPools(nextPools); setProjects(nextProjects);
        const loaded = await Promise.all(nextPools.map(async (pool) => [pool.resource_pool_id, await api.poolResources(pool.resource_pool_id)] as const));
        setPoolResources(Object.fromEntries(loaded));
      }
    } catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, [section]);

  async function run(action: () => Promise<unknown>, success: string) {
    setError(""); setNotice(""); setBusy(true);
    try { await action(); setNotice(success); await refresh(); }
    catch (reason) { setError(message(reason)); }
    finally { setBusy(false); }
  }
  async function submitUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); await run(() => api.createUser({ username, password, full_name: fullName, email }), "User created.");
    setUsername(""); setPassword(""); setFullName(""); setEmail("");
  }
  async function submitName(event: FormEvent<HTMLFormElement>, create: (value: string) => Promise<unknown>, label: string) {
    event.preventDefault(); const value = name.trim(); if (!value) return;
    await run(() => create(value), `${label} created.`); setName("");
  }

  return <PagesShell active="Access"><div className="pages-content">
    <PageTitle eyebrow="ACCESS CONTROL" title={sections.find((item) => item.id === section)?.label ?? "Access"} description="Manage controller accounts, role privileges, path-based access rules, and shared project pools." actions={<Button variant="outline" onClick={() => void refresh()} disabled={busy}>Refresh</Button>} />
    <nav className="access-tabs" aria-label="Access management sections">{sections.map((item) => <Link key={item.id} to={`/access/${item.id}` as never} className={section === item.id ? "is-active" : ""}>{item.label}</Link>)}</nav>
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}</div>}
    {section === "users" && <Card className="access-panel"><div className="access-panel-heading"><div><h2>Users</h2><p>Create accounts and update profile or activation state.</p></div></div>
      <form className="access-form-grid" onSubmit={(event) => void submitUser(event)}><label>Username<Input required minLength={3} value={username} onChange={(event) => setUsername(event.target.value)} /></label><label>Password<Input required minLength={8} type="password" value={password} onChange={(event) => setPassword(event.target.value)} /></label><label>Full name<Input value={fullName} onChange={(event) => setFullName(event.target.value)} /></label><label>Email<Input type="email" value={email} onChange={(event) => setEmail(event.target.value)} /></label><Button type="submit" disabled={busy}>Create user</Button></form>
      <div className="access-list">{users.map((user) => <UserRow key={user.user_id} user={user} run={run} />)}</div>
    </Card>}
    {section === "groups" && <Card className="access-panel"><div className="access-panel-heading"><div><h2>Groups</h2><p>Groups collect users for shared ACL assignments.</p></div></div>
      <form className="access-inline-form" onSubmit={(event) => void submitName(event, api.createGroup, "Group")}><Input required minLength={3} aria-label="New group name" placeholder="Group name" value={name} onChange={(event) => setName(event.target.value)} /><Button type="submit">Create group</Button></form>
      {groups.map((group) => <GroupRow key={group.user_group_id} group={group} users={users} members={members[group.user_group_id] ?? []} run={run} />)}
    </Card>}
    {section === "roles" && <Card className="access-panel"><div className="access-panel-heading"><div><h2>Roles</h2><p>Roles bundle privileges. Built-in roles are protected from rename and deletion.</p></div></div>
      <form className="access-inline-form" onSubmit={(event) => void submitName(event, (value) => api.createRole(value), "Role")}><Input required aria-label="New role name" placeholder="Role name" value={name} onChange={(event) => setName(event.target.value)} /><Button type="submit">Create role</Button></form>
      <div className="access-list">{roles.map((role) => <RoleRow key={role.role_id} role={role} privileges={privileges} busy={busy} run={run} />)}</div>
    </Card>}
    {section === "acl" && <Card className="access-panel"><div className="access-panel-heading"><div><h2>Access control entries</h2><p>Entries grant or deny one role on a path. Inherited entries follow the most specific matching path.</p></div></div>
      <form className="access-acl-form" onSubmit={(event) => { event.preventDefault(); const payload = { ace_type: aclType, path: aclPath, role_id: selectedRole, propagate: aclPropagate, allowed: aclAllowed, ...(aclType === "user" ? { user_id: aclPrincipal } : { group_id: aclPrincipal }) }; void run(() => api.createAcl(payload), "Access rule created."); }}>
        <label>Principal type<select value={aclType} onChange={(event) => { setAclType(event.target.value as "user" | "group"); setAclPrincipal(""); }}><option value="user">User</option><option value="group">Group</option></select></label>
        <label>{aclType === "user" ? "User" : "Group"}<select required value={aclPrincipal} onChange={(event) => setAclPrincipal(event.target.value)}><option value="">Choose…</option>{(aclType === "user" ? users : groups).map((item) => <option key={"user_id" in item ? item.user_id : item.user_group_id} value={"user_id" in item ? item.user_id : item.user_group_id}>{"username" in item ? item.username : item.name}</option>)}</select></label>
        <label>Role<select required value={selectedRole} onChange={(event) => setSelectedRole(event.target.value)}><option value="">Choose…</option>{roles.map((role) => <option key={role.role_id} value={role.role_id}>{role.name}</option>)}</select></label>
        <label>Path<Input list="access-control-paths" required value={aclPath} onChange={(event) => setAclPath(event.target.value)} /><datalist id="access-control-paths">{endpoints.map((endpoint) => <option key={endpoint.endpoint} value={endpoint.endpoint}>{endpoint.name}</option>)}{pools.map((pool) => <option key={pool.resource_pool_id} value={`/pools/${pool.resource_pool_id}`}>{pool.name}</option>)}</datalist></label>
        <label className="access-check"><input type="checkbox" checked={aclAllowed} onChange={(event) => setAclAllowed(event.target.checked)} />Allow access</label><label className="access-check"><input type="checkbox" checked={aclPropagate} onChange={(event) => setAclPropagate(event.target.checked)} />Apply to child paths</label><Button type="submit" disabled={busy || !selectedRole || !aclPrincipal}>Add entry</Button>
      </form>
      <div className="access-list">{entries.map((entry) => <AclRow key={entry.ace_id} entry={entry} users={users} groups={groups} roles={roles} run={run} />)}</div>
    </Card>}
    {section === "pools" && <Card className="access-panel"><div className="access-panel-heading"><div><h2>Resource pools</h2><p>Group projects and grant access through an ACL entry at <code>/pools/&lt;pool_id&gt;</code>.</p></div></div>
      <form className="access-inline-form" onSubmit={(event) => void submitName(event, api.createPool, "Resource pool")}><Input required aria-label="New resource pool name" placeholder="Pool name" value={name} onChange={(event) => setName(event.target.value)} /><Button type="submit">Create pool</Button></form>
      {pools.map((pool) => <PoolRow key={pool.resource_pool_id} pool={pool} projects={projects} resources={poolResources[pool.resource_pool_id] ?? []} selectedPool={selectedPool} selectedProject={selectedProject} setSelectedPool={setSelectedPool} setSelectedProject={setSelectedProject} run={run} />)}
    </Card>}
  </div></PagesShell>;
}

function RoleRow({ role, privileges, busy, run }: { role: AccessRole; privileges: AccessPrivilege[]; busy: boolean; run: (action: () => Promise<unknown>, success: string) => Promise<void> }) {
  const [name, setName] = useState(role.name);
  const [description, setDescription] = useState(role.description ?? "");
  return <div className="access-role-row" key={role.role_id}><div className="access-row"><div><small>{role.privileges.length} privileges · {role.is_builtin ? "Built-in role" : "Custom role"}</small></div><Input aria-label={`${role.name} role name`} disabled={role.is_builtin} value={name} onChange={(event) => setName(event.target.value)} /><Input aria-label={`${role.name} description`} disabled={role.is_builtin} value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Description" />{!role.is_builtin && <><Button size="sm" variant="outline" onClick={() => void run(() => api.updateRole(role.role_id, { name, description }), "Role updated.")}>Save</Button><Button size="sm" variant="destructive" onClick={() => void run(() => api.deleteRole(role.role_id), "Role deleted.")}>Delete</Button></>}</div><div className="access-privileges">{privileges.map((privilege) => { const checked = role.privileges.some((item) => item.privilege_id === privilege.privilege_id); return <label key={privilege.privilege_id}><input type="checkbox" checked={checked} disabled={role.is_builtin || busy} onChange={() => void run(() => checked ? api.removeRolePrivilege(role.role_id, privilege.privilege_id) : api.addRolePrivilege(role.role_id, privilege.privilege_id), "Role privileges updated.")} />{privilege.name}</label>; })}</div></div>;
}

function PoolRow({ pool, projects, resources, selectedPool, selectedProject, setSelectedPool, setSelectedProject, run }: { pool: ResourcePool; projects: Project[]; resources: Array<{ resource_id: string; name?: string | null }>; selectedPool: string; selectedProject: string; setSelectedPool: (value: string) => void; setSelectedProject: (value: string) => void; run: (action: () => Promise<unknown>, success: string) => Promise<void> }) {
  const [name, setName] = useState(pool.name);
  return <div className="access-pool-row"><div className="access-row"><div><small>{pool.resource_pool_id}</small></div><Input aria-label={`${pool.name} pool name`} value={name} onChange={(event) => setName(event.target.value)} /><Button size="sm" variant="outline" onClick={() => void run(() => api.updatePool(pool.resource_pool_id, name), "Pool updated.")}>Save</Button><Button size="sm" variant="destructive" onClick={() => void run(() => api.deletePool(pool.resource_pool_id), "Pool deleted.")}>Delete</Button></div><div className="access-inline-form"><select aria-label={`Project to add to ${pool.name}`} value={selectedPool === pool.resource_pool_id ? selectedProject : ""} onChange={(event) => { setSelectedPool(pool.resource_pool_id); setSelectedProject(event.target.value); }}><option value="">Add project…</option>{projects.map((project) => <option key={project.project_id} value={project.project_id}>{project.name}</option>)}</select><Button size="sm" variant="outline" disabled={selectedPool !== pool.resource_pool_id || !selectedProject} onClick={() => void run(() => api.addPoolResource(pool.resource_pool_id, selectedProject), "Project added to pool.")}>Add project</Button></div><div className="access-pool-resources">{resources.map((resource) => <div key={resource.resource_id}><span>{resource.name ?? resource.resource_id}</span><Button size="sm" variant="ghost" onClick={() => void run(() => api.removePoolResource(pool.resource_pool_id, resource.resource_id), "Project removed from pool.")}>Remove</Button></div>)}</div></div>;
}

function UserRow({ user, run }: { user: AccessUser; run: (action: () => Promise<unknown>, success: string) => Promise<void> }) {
  const [fullName, setFullName] = useState(user.full_name ?? "");
  const [email, setEmail] = useState(user.email ?? "");
  return <div className="access-row access-user-row"><div><strong>{user.username}{user.is_superadmin ? " · Super administrator" : ""}</strong><small>{user.is_active ? "Active" : "Disabled"}</small></div><Input aria-label={`${user.username} full name`} value={fullName} onChange={(event) => setFullName(event.target.value)} placeholder="Full name" /><Input aria-label={`${user.username} email`} type="email" value={email} onChange={(event) => setEmail(event.target.value)} placeholder="Email" /><Button size="sm" variant="outline" onClick={() => void run(() => api.updateUser(user.user_id, { full_name: fullName, email }), "User updated.")}>Save</Button><Button size="sm" variant="outline" onClick={() => void run(() => api.updateUser(user.user_id, { is_active: !user.is_active }), user.is_active ? "User disabled." : "User activated.")}>{user.is_active ? "Disable" : "Enable"}</Button>{!user.is_superadmin && <Button size="sm" variant="destructive" onClick={() => void run(() => api.deleteUser(user.user_id), "User deleted.")}>Delete</Button>}</div>;
}

function GroupRow({ group, users, members, run }: { group: AccessGroup; users: AccessUser[]; members: AccessUser[]; run: (action: () => Promise<unknown>, success: string) => Promise<void> }) {
  const [name, setName] = useState(group.name);
  const [memberId, setMemberId] = useState("");
  return <div className="access-group-row"><div className="access-row"><div><strong>{group.name}</strong><small>{group.is_builtin ? "Built-in group" : `${members.length} members`}</small></div><Input aria-label={`${group.name} name`} disabled={group.is_builtin} value={name} onChange={(event) => setName(event.target.value)} /><Button size="sm" variant="outline" disabled={group.is_builtin} onClick={() => void run(() => api.updateGroup(group.user_group_id, name), "Group updated.")}>Save</Button><Button size="sm" variant="destructive" disabled={group.is_builtin} onClick={() => void run(() => api.deleteGroup(group.user_group_id), "Group deleted.")}>Delete</Button></div><div className="access-inline-form"><select aria-label={`Add member to ${group.name}`} value={memberId} onChange={(event) => setMemberId(event.target.value)}><option value="">Add member…</option>{users.filter((user) => !members.some((member) => member.user_id === user.user_id)).map((user) => <option key={user.user_id} value={user.user_id}>{user.username}</option>)}</select><Button size="sm" variant="outline" disabled={!memberId} onClick={() => void run(() => api.addGroupMember(group.user_group_id, memberId), "Group membership updated.")}>Add</Button>{members.map((member) => <span className="access-member-chip" key={member.user_id}>{member.username}<button aria-label={`Remove ${member.username} from ${group.name}`} onClick={() => void run(() => api.removeGroupMember(group.user_group_id, member.user_id), "Group membership updated.")}>×</button></span>)}</div></div>;
}

function AclRow({ entry, users, groups, roles, run }: { entry: AccessControlEntry; users: AccessUser[]; groups: AccessGroup[]; roles: AccessRole[]; run: (action: () => Promise<unknown>, success: string) => Promise<void> }) {
  const [path, setPath] = useState(entry.path);
  const principal = entry.ace_type === "user" ? users.find((user) => user.user_id === entry.user_id)?.username : groups.find((group) => group.user_group_id === entry.group_id)?.name;
  const role = roles.find((item) => item.role_id === entry.role_id)?.name ?? entry.role_id;
  return <div className="access-row access-acl-row"><div><strong>{principal ?? "Unknown principal"} · {role}</strong><small>{entry.ace_type} · {entry.allowed ? "Allowed" : "Denied"} · {entry.propagate ? "Inherited" : "This path only"}</small></div><Input aria-label="ACL path" value={path} onChange={(event) => setPath(event.target.value)} /><Button size="sm" variant="outline" onClick={() => void run(() => api.updateAcl(entry.ace_id, { path }), "ACL path updated.")}>Save path</Button><Button size="sm" variant="outline" onClick={() => void run(() => api.updateAcl(entry.ace_id, { allowed: !entry.allowed }), entry.allowed ? "Entry changed to deny." : "Entry changed to allow.")}>{entry.allowed ? "Deny" : "Allow"}</Button><Button size="sm" variant="outline" onClick={() => void run(() => api.updateAcl(entry.ace_id, { propagate: !entry.propagate }), "Inheritance updated.")}>{entry.propagate ? "Stop inheritance" : "Propagate"}</Button><Button size="sm" variant="destructive" onClick={() => void run(() => api.deleteAcl(entry.ace_id), "Entry deleted.")}>Delete</Button></div>;
}
