import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api, getServerOrigin, isDesktop } from "../api";
import { AppLogo } from "@/components/app-logo";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

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
  return <main className="flex min-h-svh items-center justify-center bg-muted/40 p-6">
    <Card className="w-full max-w-sm">
      <CardHeader className="text-center">
        <AppLogo className="mx-auto mb-2 size-14" />
        <p className="text-xs font-medium tracking-wide text-muted-foreground">NETWORK STUDIO</p>
        <CardTitle className="text-2xl font-semibold tracking-tight">Sign in</CardTitle>
        <CardDescription>{isDesktop() ? <>Signing in to <strong className="font-medium text-foreground">{getServerOrigin()}</strong>. <a className="underline underline-offset-4" href="/connect">Change server</a></> : "Use your controller account to continue."}</CardDescription>
      </CardHeader>
      <CardContent>
      <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
        <Label htmlFor="login-username">Username</Label>
        <Input id="login-username" autoComplete="username" required value={username} onChange={(event) => setUsername(event.target.value)} />
        <Label htmlFor="login-password">Password</Label>
        <Input id="login-password" type="password" autoComplete="current-password" required value={password} onChange={(event) => setPassword(event.target.value)} />
        {error && <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert>}
        <Button type="submit" className="w-full" disabled={busy}>{busy ? "Signing in…" : "Sign in"}</Button>
      </form>
      </CardContent>
    </Card>
  </main>;
}
