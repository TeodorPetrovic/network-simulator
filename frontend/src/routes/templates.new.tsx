import { createFileRoute } from "@tanstack/react-router";
import { TemplateWizardPage } from "../TemplateWizardPage";

export const Route = createFileRoute("/templates/new")({ component: TemplateWizardPage });
