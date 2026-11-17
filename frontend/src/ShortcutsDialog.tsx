import { useEffect } from "react";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import { SHORTCUTS } from "./shortcuts";

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA";
}

// Calls onOpen when "?" is pressed, unless an input, textarea or contenteditable element has focus.
export function useShortcutsHotkey(onOpen: () => void) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "?" || event.ctrlKey || event.metaKey || event.altKey) return;
      if (isEditableTarget(event.target) || isEditableTarget(document.activeElement)) return;
      event.preventDefault();
      onOpen();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onOpen]);
}

export function ShortcutsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const groups = [...new Set(SHORTCUTS.map((shortcut) => shortcut.group))];
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
    <DialogHeader><DialogTitle>Keyboard shortcuts</DialogTitle><DialogDescription>Shortcuts available in the project workspace. Press ? to open this list.</DialogDescription></DialogHeader>
    {groups.map((group) => <section key={group} className="grid gap-2">
      <h3 className="text-sm font-semibold">{group}</h3>
      <Table><TableBody>{SHORTCUTS.filter((shortcut) => shortcut.group === group).map((shortcut) => <TableRow key={shortcut.description}>
        <TableCell className="w-[40%] align-top"><span className="flex flex-wrap gap-1">{shortcut.keys.map((key) => <kbd key={key} className="rounded border bg-muted px-1.5 py-0.5 font-mono text-xs">{key}</kbd>)}</span></TableCell>
        <TableCell className="align-top text-sm">{shortcut.description}</TableCell>
      </TableRow>)}</TableBody></Table>
    </section>)}
    <DialogFooter><Button onClick={() => onOpenChange(false)}>Close</Button></DialogFooter>
  </DialogContent></Dialog>;
}
