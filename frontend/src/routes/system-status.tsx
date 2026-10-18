import { createFileRoute } from "@tanstack/react-router";
import { SystemStatusPage } from "../SystemStatusPage";

export const Route = createFileRoute("/system-status")({ component: SystemStatusPage });
