import { createFileRoute } from "@tanstack/react-router";
import { PreferencesPage } from "../PreferencesPage";

export const Route = createFileRoute("/preferences")({
  validateSearch: (search: Record<string, unknown>) => ({
    fromProject: typeof search.fromProject === "string" ? search.fromProject : undefined,
  }),
  component: PreferencesRoute,
});

function PreferencesRoute() {
  const { fromProject } = Route.useSearch();
  return <PreferencesPage fromProject={fromProject} />;
}
