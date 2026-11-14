import { Brand } from "@/components/brand";
import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { cn } from "@/lib/utils";
import { Activity, Boxes, Cpu, FolderOpen, HardDrive, Network, Settings2, ShieldCheck, UserRound } from "lucide-react";

const sections = [
  { to: "/", label: "Projects", icon: FolderOpen },
  { to: "/preferences", label: "Preferences", icon: Boxes },
  { to: "/appliances", label: "Appliances", icon: HardDrive },
  { to: "/computes", label: "Servers", icon: Network },
  { to: "/settings", label: "Settings", icon: Settings2 },
  { to: "/access/users", label: "Access", icon: ShieldCheck },
  { to: "/user-settings", label: "My account", icon: UserRound },
  { to: "/system-status", label: "System status", icon: Activity },
  { to: "/installed-software", label: "Installed software", icon: Cpu },
] as const;

export function PagesShell({ children, active, fromProject }: { children: ReactNode; active: string; fromProject?: string }) {
  return <div className="flex h-dvh min-h-0 flex-col overflow-hidden">
    <header className="flex h-14 min-h-14 shrink-0 items-center gap-5 border-b bg-background px-4 sm:px-6">
      <Link to="/" className="inline-flex shrink-0 items-center text-foreground no-underline"><Brand /></Link>
      <nav className="flex min-w-0 items-center gap-1 overflow-x-auto" aria-label="Application pages">
        {sections.map(({ to, label, icon: Icon }) => {
          const cls = cn("inline-flex h-8 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-xs font-medium text-muted-foreground no-underline transition-colors hover:bg-accent hover:text-accent-foreground", active === label && "bg-accent text-accent-foreground");
          return to === "/appliances"
            ? <Link key={to} to="/appliances" search={{ fromProject }} title={label} className={cls} aria-current={active === label ? "page" : undefined}><Icon size={15} /><span className="max-[1100px]:sr-only">{label}</span></Link>
            : <Link key={to} to={to as never} title={label} className={cls} aria-current={active === label ? "page" : undefined}><Icon size={15} /><span className="max-[1100px]:sr-only">{label}</span></Link>;
        })}
      </nav>
    </header>
    <main className="min-h-0 flex-1 overflow-y-auto overscroll-contain" data-page-scroll-region>{children}</main>
  </div>;
}

/** Shared constrained content frame for pages that live inside the application shell. */
export function PageLayout({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`mx-auto w-full max-w-7xl px-4 py-6 sm:px-6 lg:px-8 ${className}`}>{children}</div>;
}

/** A consistent section boundary for related page content. */
export function PageSection({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <section className={`mt-6 min-w-0 ${className}`}>{children}</section>;
}

/** Left-hand navigation frame for account and settings pages. */
export function SettingsLayout({ children, navigation }: { children: ReactNode; navigation: ReactNode }) {
  return <div className="grid w-full grid-cols-1 gap-6 lg:grid-cols-[13rem_minmax(0,1fr)]">
    <nav aria-label="Settings" className="min-w-0">{navigation}</nav>
    <div className="min-w-0">{children}</div>
  </div>;
}

export function PageTitle({ eyebrow, title, description, actions }: { eyebrow?: string; title: string; description?: string; actions?: ReactNode }) {
  return <div className="mb-6 flex flex-wrap items-start justify-between gap-4"><div className="min-w-0">{eyebrow && <div className="mb-1 text-xs font-medium tracking-wide text-muted-foreground">{eyebrow}</div>}<h1 className="text-2xl font-semibold tracking-tight">{title}</h1>{description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}</div>{actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}</div>;
}
