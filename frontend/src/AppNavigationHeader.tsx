import { Network } from "lucide-react";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

type Props = { connection: SimulatorWorkspaceProps["connection"]; onNavigateHome: () => void };

export function AppNavigationHeader({ connection, onNavigateHome }: Props) {
  return (
        <header className="sim-app-header">
          <Button variant="ghost" className="sim-brand" onClick={onNavigateHome}>
            <span className="sim-brand-mark"><Network size={18} /></span>
            <span>Network Simulator</span>
          </Button>
          <div className="sim-header-actions">
            <Badge variant={connection === "connected" ? "secondary" : connection === "disconnected" ? "destructive" : "outline"} className="gap-1.5">
              <span className={`sim-connection-dot ${connection}`} />{connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable"}
            </Badge>
          </div>
        </header>
  );
}
