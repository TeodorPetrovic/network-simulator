import { createFileRoute } from "@tanstack/react-router";
import { BuiltinNodeEditorPage } from "../BuiltinNodeEditorPage";

export const Route = createFileRoute("/node-editors/$nodeType")({ component: NodeEditorRoute });

function NodeEditorRoute() {
  const { nodeType } = Route.useParams();
  return <BuiltinNodeEditorPage nodeType={nodeType} />;
}
