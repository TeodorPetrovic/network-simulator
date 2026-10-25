import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "./App.test-utils";
import { TemplateSettingsDialog } from "./TemplateSettingsDialog";

const template = {
  template_id: "router-1",
  name: "Cisco 1700",
  template_type: "dynamips" as const,
  platform: "c1700" as const,
  image: "router.bin",
  adapters: 1 as const,
  memory: 160,
  nvram: 128,
  idlepc: "0x80358a60",
};

describe("TemplateSettingsDialog", () => {
  it("shows the current Dynamips platform and all supported options in an aligned popup", async () => {
    render(<TemplateSettingsDialog template={template} open onOpenChange={vi.fn()} onSave={vi.fn()} />);

    const trigger = screen.getByRole("combobox", { name: "Dynamips platform" });
    expect(trigger).toHaveTextContent("c1700");
    fireEvent.pointerDown(trigger);
    fireEvent.click(trigger);

    for (const platform of ["c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200"]) {
      expect(await screen.findByRole("option", { name: platform })).toBeInTheDocument();
    }
    expect(trigger).toHaveTextContent("c1700");
  });
});
