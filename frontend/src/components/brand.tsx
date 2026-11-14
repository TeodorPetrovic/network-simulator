import { AppLogo } from "@/components/app-logo";
import { cn } from "@/lib/utils";

/** The product mark used by every header so the app looks the same on every page. */
export function Brand({ className, showName = true }: { className?: string; showName?: boolean }) {
  return <span className={cn("inline-flex shrink-0 items-center gap-2 font-semibold tracking-tight", className)}><AppLogo className="size-7" />{showName && <span>Network Studio</span>}</span>;
}
