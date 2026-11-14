import { useEffect } from "react";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";

type GlobalToastProps = {
  error: string;
  notice: string;
  onConsumed: () => void;
};

/** Turns the app-level `error` / `notice` strings into shadcn (Sonner) toasts. */
export function GlobalToast({ error, notice, onConsumed }: GlobalToastProps) {
  useEffect(() => {
    if (!error && !notice) return;
    if (error) toast.error(error, { duration: 8000 });
    if (notice) toast(notice, { duration: 5000 });
    onConsumed();
  }, [error, notice, onConsumed]);

  return <Toaster />;
}
