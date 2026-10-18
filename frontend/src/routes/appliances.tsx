import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { AppRoute } from "../AppRoute";

export const Route = createFileRoute("/appliances")({
  validateSearch: (search: Record<string, unknown>) => ({
    fromProject: typeof search.fromProject === "string" ? search.fromProject : undefined,
  }),
  component: ApplianceManagerRoute,
});

function ApplianceManagerRoute() {
  const { fromProject } = Route.useSearch();
  const navigate = useNavigate();
  const back = () => {
    if (fromProject) void navigate({ to: "/projects/$projectId", params: { projectId: fromProject } });
    else void navigate({ to: "/" });
  };
  return <AppRoute projectId={null} returnProjectId={fromProject} showApplianceManager onBackFromApplianceManager={back} />;
}
