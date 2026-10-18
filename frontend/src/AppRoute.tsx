import { useNavigate } from "@tanstack/react-router";
import App from "./App";

export function AppRoute({ projectId, returnProjectId, showApplianceManager = false, onBackFromApplianceManager }: { projectId: string | null; returnProjectId?: string; showApplianceManager?: boolean; onBackFromApplianceManager?: () => void }) {
  const navigate = useNavigate();

  function navigateToProject(id: string | null) {
    if (id) {
      void navigate({ to: "/projects/$projectId", params: { projectId: id } });
    } else {
      void navigate({ to: "/" });
    }
  }

  function navigateToApplianceManager() {
    void navigate({ to: "/appliances", search: { fromProject: projectId ?? undefined } });
  }

  function navigateToPreferences() {
    void navigate({ to: "/preferences" as never, search: { fromProject: projectId ?? returnProjectId } as never });
  }

  return <App projectId={projectId} navigateToProject={navigateToProject} showApplianceManager={showApplianceManager} navigateToApplianceManager={navigateToApplianceManager} onOpenPreferences={navigateToPreferences} onBackFromApplianceManager={onBackFromApplianceManager} />;
}
