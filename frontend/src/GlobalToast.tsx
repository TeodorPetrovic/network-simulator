import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

type GlobalToastProps = {
  error: string;
  notice: string;
  onDismiss: () => void;
};

export function GlobalToast({ error, notice, onDismiss }: GlobalToastProps) {
  if (!error && !notice) return null;

  return (
    <Card className={`fixed bottom-[18px] right-[18px] z-[100] flex max-h-32 max-w-[min(560px,calc(100vw-36px))] items-center gap-2.5 px-3 py-2 text-xs shadow-lg ${error ? "border-destructive text-destructive" : ""}`} role={error ? "alert" : "status"} aria-live="polite">
      <span className="max-h-24 overflow-auto">{error || notice}</span>
      <Button variant="ghost" size="icon-xs" onClick={onDismiss} aria-label="Dismiss notification"><X /></Button>
    </Card>
  );
}
