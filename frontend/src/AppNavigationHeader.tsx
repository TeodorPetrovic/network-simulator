import { AppLogo } from "@/components/app-logo";
import type { SimulatorWorkspaceProps } from "./SimulatorWorkspace";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

type Props = { connection: SimulatorWorkspaceProps["connection"]; onNavigateHome: () => void };

export function AppNavigationHeader({ connection, onNavigateHome }: Props) {
  return (
        <header className="z-10 flex h-[52px] min-h-[52px] items-center gap-5 border-b bg-background px-3.5">
          <Button variant="ghost" className="inline-flex items-center gap-2 font-semibold" onClick={onNavigateHome}>
            <AppLogo className="size-7" />
            <span>Network Studio</span>
          </Button>
          <div className="ml-auto flex items-center gap-2.5">
            <Badge variant={connection === "connected" ? "secondary" : connection === "disconnected" ? "destructive" : "outline"} className="gap-1.5">
              <span className={`size-1.5 rounded-full ${connection === "connected" ? "bg-success" : connection === "reconnecting" || connection === "checking" ? "bg-warning" : "bg-destructive"}`} />{connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable"}
            </Badge>
          </div>
        </header>
  );
}
