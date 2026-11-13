import { createFileRoute, useSearch } from "@tanstack/react-router";
import { ConsoleSession } from "../ConsoleDock";
import type { ConsoleTarget } from "../ConsoleDock";

export const Route = createFileRoute("/console")({
  validateSearch: (search: Record<string, unknown>) => ({
    projectId: String(search.projectId ?? ""),
    nodeId: String(search.nodeId ?? ""),
    nodeName: String(search.nodeName ?? "Console"),
    nodeType: String(search.nodeType ?? "vpcs"),
    auxiliary: search.auxiliary === "1" || search.auxiliary === 1 || search.auxiliary === true,
  }),
  component: ConsoleRoute,
});

function ConsoleRoute() {
  const q = useSearch({ from: "/console" });
  const target: ConsoleTarget = { id: q.nodeId, projectId: q.projectId, nodeId: q.nodeId, nodeName: q.nodeName, nodeType: q.nodeType as ConsoleTarget["nodeType"], auxiliary: q.auxiliary };
  return <main className="fixed inset-0 flex flex-col bg-background text-foreground">
    <div className="relative flex min-h-0 flex-1"><ConsoleSession target={target} active /></div>
  </main>;
}
