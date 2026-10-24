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
    <Card className={`sim-flash ${error ? "sim-flash-error" : ""}`} role={error ? "alert" : "status"} aria-live="polite">
      <span>{error || notice}</span>
      <Button variant="ghost" size="icon-xs" onClick={onDismiss} aria-label="Dismiss notification"><X /></Button>
    </Card>
  );
}
