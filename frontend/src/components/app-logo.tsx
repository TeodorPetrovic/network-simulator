import { cn } from "@/lib/utils";
import iconUrl from "@/assets/icon.svg";

export { iconUrl };

/** The Network Studio mark, identical to the desktop app icon (src/assets/icon.svg). Use it everywhere the app is branded. */
export function AppLogo({ className }: { className?: string }) {
  return <img src={iconUrl} alt="" aria-hidden="true" draggable={false} className={cn("size-8 shrink-0 select-none", className)} />;
}
