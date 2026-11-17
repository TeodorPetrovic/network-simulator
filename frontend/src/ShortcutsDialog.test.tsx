import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import "./App.test-utils";
import { emptyTopology, renderWorkspace } from "./App.test-utils";
import { api } from "./api";
import { ShortcutsDialog, useShortcutsHotkey } from "./ShortcutsDialog";
import { SHORTCUTS } from "./shortcuts";

function ShortcutsHost() {
  const [open, setOpen] = useState(false);
  useShortcutsHotkey(() => setOpen(true));
  return <>
    <input aria-label="Filter" />
    <ShortcutsDialog open={open} onOpenChange={setOpen} />
  </>;
}

describe("ShortcutsDialog", () => {
  it("lists every entry of the shortcuts constant", () => {
    render(<ShortcutsDialog open onOpenChange={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    for (const shortcut of SHORTCUTS) {
      const row = screen.getByText(shortcut.description).closest("tr");
      expect(row, shortcut.description).not.toBeNull();
      for (const key of shortcut.keys) {
        expect(within(row as HTMLElement).getByText(key, { selector: "kbd" })).toBeInTheDocument();
      }
    }
  });

  it("opens with the question mark key when no field has focus", () => {
    render(<ShortcutsHost />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.keyDown(document.body, { key: "?" });
    expect(screen.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
  });

  it("ignores the question mark key typed inside an input", () => {
    render(<ShortcutsHost />);
    fireEvent.keyDown(screen.getByLabelText("Filter"), { key: "?" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("opens from the Help menu item in the project workspace", async () => {
    vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes: [] });
    renderWorkspace();
    const helpMenu = await screen.findByRole("menuitem", { name: "Help menu" });
    fireEvent.pointerDown(helpMenu, { button: 0, ctrlKey: false, pointerType: "mouse" });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Keyboard shortcuts" }));
    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
  });
});
