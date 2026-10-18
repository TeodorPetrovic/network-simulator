import { createFileRoute } from "@tanstack/react-router";
import { SnapshotsPage } from "../SnapshotsPage";

export const Route = createFileRoute("/projects/$projectId/snapshots")({ component: ProjectSnapshotsRoute });

function ProjectSnapshotsRoute() {
  const { projectId } = Route.useParams();
  return <SnapshotsPage projectId={projectId} />;
}
