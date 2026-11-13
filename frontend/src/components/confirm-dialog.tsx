import { useCallback, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

type ConfirmOptions = { title?: string; description: ReactNode; confirmLabel?: string; destructive?: boolean };

/**
 * Promise-based replacement for the browser's blocking confirm prompt.
 *   const [confirm, confirmDialog] = useConfirm();
 *   if (!(await confirm({ description: "Delete X?", destructive: true }))) return;
 * Render `{confirmDialog}` once in the component tree.
 */
export function useConfirm() {
  const [options, setOptions] = useState<ConfirmOptions | null>(null);
  const resolver = useRef<((value: boolean) => void) | null>(null);

  const confirm = useCallback((next: ConfirmOptions) => new Promise<boolean>((resolve) => {
    resolver.current?.(false);
    resolver.current = resolve;
    setOptions(next);
  }), []);

  const settle = (value: boolean) => {
    resolver.current?.(value);
    resolver.current = null;
    setOptions(null);
  };

  const dialog = <Dialog open={options !== null} onOpenChange={(open) => { if (!open) settle(false); }}>
    <DialogContent showCloseButton={false} className="max-w-md">
      <DialogHeader>
        <DialogTitle>{options?.title ?? "Are you sure?"}</DialogTitle>
        <DialogDescription>{options?.description}</DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={() => settle(false)}>Cancel</Button>
        <Button type="button" variant={options?.destructive ? "destructive" : "default"} onClick={() => settle(true)}>{options?.confirmLabel ?? "Confirm"}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>;

  return [confirm, dialog] as const;
}
