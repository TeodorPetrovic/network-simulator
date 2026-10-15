import { useNavigate } from "@tanstack/react-router";
import App from "./App";

export function AppRoute({ projectId }: { projectId: string | null }) {
  const navigate = useNavigate();

  function navigateToProject(id: string | null) {
    if (id) {
      void navigate({ to: "/projects/$projectId", params: { projectId: id } });
    } else {
      void navigate({ to: "/" });
    }
  }

  return <App projectId={projectId} navigateToProject={navigateToProject} />;
}
