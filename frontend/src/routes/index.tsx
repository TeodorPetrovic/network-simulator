import { useEffect } from "react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { ProjectsPage } from "../ProjectsPage";

export const Route = createFileRoute("/")({
  component: ProjectsRoute,
});

function ProjectsRoute() {
  const navigate = useNavigate();

  useEffect(() => {
    const match = window.location.hash.match(/^#\/projects\/([^/]+)$/);
    const projectId = match?.[1];
    if (projectId) {
      void navigate({ to: "/projects/$projectId", params: { projectId } });
    }
  }, [navigate]);

  return <ProjectsPage />;
}
