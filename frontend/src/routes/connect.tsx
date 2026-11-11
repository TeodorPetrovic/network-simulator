import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { getServerOrigin, isDesktop, normalizeServerUrl, recentServers, setServerOrigin } from "../api";
import { saveAuthToken } from "../api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export const Route = createFileRoute("/connect")({ component: ConnectPage });

/** Probe a controller before saving it, so a typo is reported here rather than as a broken login. */
export async function probeServer(input: string, fetcher: typeof fetch = fetch): Promise<string> {
  const origin = normalizeServerUrl(input);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    const response = await fetcher(`${origin}/healthz`, { signal: controller.signal });
    if (!response.ok) throw new Error(`The server answered ${response.status}. Is this a Network Studio controller?`);
  } catch (reason) {
    if (reason instanceof Error && reason.message.startsWith("The server answered")) throw reason;
    throw new Error(`Could not reach ${origin}. Check the address and that the controller is running and allows desktop clients (it must be a build with desktop support).`);
  } finally { clearTimeout(timer); }
  return origin;
}

function ConnectPage() {
  const navigate = useNavigate();
  const [address, setAddress] = useState(getServerOrigin());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const recent = recentServers();
  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); setBusy(true); setError("");
    try {
      const origin = await probeServer(address);
      if (origin !== getServerOrigin()) saveAuthToken(null);
      setServerOrigin(origin);
      await navigate({ to: "/login", search: { returnTo: "/" }, replace: true });
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Could not connect"); }
    finally { setBusy(false); }
  }
  return <main className="login-page">
    <section className="login-card">
      <div className="login-mark">NS</div>
      <div className="pages-eyebrow">NETWORK STUDIO</div>
      <h1>Connect to a server</h1>
      <p>{isDesktop()
        ? "Devices and consoles run on the server you choose; this app is only the interface."
        : "This page is for the desktop app. In a browser the controller that served it is used automatically."}</p>
      <form className="login-form" onSubmit={(event) => void submit(event)}>
        <label htmlFor="server-address">Server address</label>
        <Input id="server-address" placeholder="lab-server:3080 or https://lab.example.com" autoFocus required value={address} onChange={(event) => setAddress(event.target.value)} />
        {recent.length > 0 && <div className="flex flex-wrap gap-2" aria-label="Recent servers">
          {recent.map((entry) => <Button key={entry} type="button" variant="outline" size="sm" onClick={() => setAddress(entry)}>{entry}</Button>)}
        </div>}
        {error && <div role="alert" className="pages-alert">{error}</div>}
        <Button type="submit" className="w-full" disabled={busy}>{busy ? "Connecting…" : "Connect"}</Button>
      </form>
    </section>
  </main>;
}
