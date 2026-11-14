import { Brand } from "@/components/brand";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { Badge } from "@/components/ui/badge";

type Props = { connection: SimulatorWorkspaceProps["connection"]; onNavigateHome: () => void };

export function AppNavigationHeader({ connection, onNavigateHome }: Props) {
  return (
        <header className="z-10 flex h-11 min-h-11 items-center gap-3 border-b bg-background px-3">
          <button type="button" className="inline-flex shrink-0 items-center rounded-md text-foreground" onClick={onNavigateHome}><Brand /></button>
          <div className="ml-auto flex items-center gap-2.5">
            <Badge variant={connection === "connected" ? "secondary" : connection === "disconnected" ? "destructive" : "outline"} className="gap-1.5">
              <span className={`size-1.5 rounded-full ${connection === "connected" ? "bg-success" : connection === "reconnecting" || connection === "checking" ? "bg-warning" : "bg-destructive"}`} />{connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable"}
            </Badge>
          </div>
        </header>
  );
}
