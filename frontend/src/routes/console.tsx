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
  return <main className="sim-console-dock" style={{ position: "fixed", inset: 0, width: "auto", height: "auto", borderRadius: 0, border: 0 }}>
    <div className="sim-console-panes"><ConsoleSession target={target} active /></div>
  </main>;
}
