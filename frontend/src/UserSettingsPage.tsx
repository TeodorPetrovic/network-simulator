import { useEffect, useState } from "react";
import type { AccessUser, UserApiKey } from "./api";
import { api } from "./api";
import { PageLayout, PagesShell, PageTitle } from "./PagesShell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";

export function UserSettingsPage() {
  const [user, setUser] = useState<AccessUser | null>(null);
  const [keys, setKeys] = useState<UserApiKey[]>([]);
  const [password, setPassword] = useState("");
  const [newKey, setNewKey] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  async function refresh() {
    setError("");
    try {
      const profile = await api.me();
      setUser(profile);
      setKeys(profile.must_change_password ? [] : await api.apiKeys());
    }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Account data could not be loaded."); }
  }
  useEffect(() => { void refresh(); }, []);
  async function saveProfile(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); setError(""); setNotice("");
    try { const changingPassword = Boolean(password); setUser(await api.updateMe({ full_name: user?.full_name ?? "", email: user?.email ?? "", ...(password ? { password } : {}) })); setPassword(""); setNotice(changingPassword ? "Password changed. Sign in again with the new password." : "Account details saved."); if (changingPassword) window.location.assign("/login"); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Account could not be updated."); }
  }
  async function createKey(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); setError(""); setNotice("");
    try { const created = await api.createApiKey(name); setNewKey(created.api_key); setName(""); await refresh(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "API key could not be created."); }
  }
  async function updateKey(key: UserApiKey, action: "revoke" | "restore" | "delete") {
    setError("");
    try {
      if (action === "revoke") await api.revokeApiKey(key.api_key_id);
      else if (action === "restore") await api.restoreApiKey(key.api_key_id);
      else await api.deleteApiKey(key.api_key_id);
      await refresh();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "API key could not be changed."); }
  }
  return <PagesShell active="My account"><PageLayout>
    <PageTitle eyebrow="ACCOUNT" title="User settings" description="Update your profile and password, then manage your personal API keys." actions={<Button variant="outline" onClick={() => void api.logout()}>Sign out</Button>} />
    {error && <div role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">{error}</div>}{notice && <div role="status" className="rounded-md border border-border bg-muted px-4 py-3 text-sm">{notice}</div>}
    {user?.must_change_password && <div role="alert" className="rounded-md border border-border bg-muted px-4 py-3 text-sm">Set a new password before using this account.</div>}
    {newKey && <Card className="p-4"><p className="font-medium">Copy this API key now. It is only shown once.</p><code className="block break-all pt-2">{newKey}</code><Button size="sm" variant="outline" className="mt-3" onClick={() => setNewKey("")}>Dismiss</Button></Card>}
    <div className="grid min-w-0 gap-6 lg:grid-cols-2">
      <Card className="min-w-0 space-y-4 p-5"><h2 className="text-lg font-semibold">Profile</h2><p className="text-sm text-muted-foreground">{user?.username ?? "Loading account…"}{user?.is_superadmin ? " · Super administrator" : ""}</p>
        <form className="grid gap-4" onSubmit={(event) => void saveProfile(event)}>
          <label>Full name<Input value={user?.full_name ?? ""} onChange={(event) => setUser((current) => current ? { ...current, full_name: event.target.value } : current)} /></label>
          <label>Email<Input type="email" value={user?.email ?? ""} onChange={(event) => setUser((current) => current ? { ...current, email: event.target.value } : current)} /></label>
          <label>{user?.must_change_password ? "New password (required)" : "New password"}<Input type="password" autoComplete="new-password" minLength={8} required={Boolean(user?.must_change_password)} value={password} onChange={(event) => setPassword(event.target.value)} placeholder={user?.must_change_password ? "Choose a new password" : "Leave blank to keep current password"} /></label>
          <Button type="submit">{user?.must_change_password ? "Set new password" : "Save changes"}</Button>
        </form>
      </Card>
      {!user?.must_change_password && <Card className="min-w-0 space-y-4 p-5"><h2 className="text-lg font-semibold">API keys</h2><p className="text-sm text-muted-foreground">Keys work as bearer credentials and are shown only when created.</p>
        <form className="flex flex-wrap items-end gap-3" onSubmit={(event) => void createKey(event)}><Input aria-label="API key name" placeholder="Key name" required value={name} onChange={(event) => setName(event.target.value)} /><Button type="submit">Create key</Button></form>
        <div className="divide-y rounded-md border">{keys.map((key) => <div className="grid min-w-0 items-center gap-3 p-3 sm:grid-cols-[minmax(0,1fr)_auto]" key={key.api_key_id}><div><strong>{key.name}</strong><span className="text-xs text-muted-foreground">{key.key_prefix} · {key.revoked ? "Revoked" : "Active"}</span></div><div className="flex flex-wrap gap-2">{key.revoked ? <Button size="sm" variant="outline" onClick={() => void updateKey(key, "restore")}>Restore</Button> : <Button size="sm" variant="outline" onClick={() => void updateKey(key, "revoke")}>Revoke</Button>}<Button size="sm" variant="destructive" onClick={() => void updateKey(key, "delete")}>Delete</Button></div></div>)}</div>
      </Card>}
    </div>
  </PageLayout></PagesShell>;
}
