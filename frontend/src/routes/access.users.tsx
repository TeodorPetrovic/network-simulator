import { createFileRoute } from "@tanstack/react-router";
import { AccessManagementPage } from "../AccessManagementPage";
export const Route = createFileRoute("/access/users")({ component: () => <AccessManagementPage section="users" /> });
