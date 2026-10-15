import { createFileRoute } from "@tanstack/react-router";
import { AppRoute } from "../AppRoute";

export const Route = createFileRoute("/projects/$projectId")({
  component: ProjectRoute,
});

function ProjectRoute() {
  const { projectId } = Route.useParams();
  return <AppRoute projectId={projectId} />;
}
