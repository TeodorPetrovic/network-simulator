import { createFileRoute } from "@tanstack/react-router";
import { UserSettingsPage } from "../UserSettingsPage";
export const Route = createFileRoute("/user-settings")({ component: UserSettingsPage });
