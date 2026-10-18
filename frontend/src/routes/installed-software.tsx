import { createFileRoute } from "@tanstack/react-router";
import { InstalledSoftwarePage } from "../InstalledSoftwarePage";

export const Route = createFileRoute("/installed-software")({ component: InstalledSoftwarePage });
