import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api } from "../api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export const Route = createFileRoute("/login")({
  validateSearch: (search: Record<string, unknown>) => ({ returnTo: typeof search.returnTo === "string" ? search.returnTo : "/" }),
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
  const { returnTo } = Route.useSearch();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); setBusy(true); setError("");
    try {
      const session = await api.login(username, password);
      await navigate({ to: session.must_change_password ? "/user-settings" : returnTo as never, replace: true });
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Login failed"); }
    finally { setBusy(false); }
  }
  return <main className="login-page">
    <section className="login-card">
      <div className="login-mark">NS</div>
      <div className="pages-eyebrow">NETWORK STUDIO</div>
      <h1>Sign in</h1>
      <p>Use your controller account to continue.</p>
      <form className="login-form" onSubmit={(event) => void submit(event)}>
        <label htmlFor="login-username">Username</label>
        <Input id="login-username" autoComplete="username" required value={username} onChange={(event) => setUsername(event.target.value)} />
        <label htmlFor="login-password">Password</label>
        <Input id="login-password" type="password" autoComplete="current-password" required value={password} onChange={(event) => setPassword(event.target.value)} />
        {error && <div role="alert" className="pages-alert">{error}</div>}
        <Button type="submit" className="w-full" disabled={busy}>{busy ? "Signing in…" : "Sign in"}</Button>
      </form>
    </section>
  </main>;
}
