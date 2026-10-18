import { createFileRoute } from "@tanstack/react-router";
import { ComputesPage } from "../ComputesPage";

export const Route = createFileRoute("/computes")({ component: ComputesPage });
