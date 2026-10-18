import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Activity, Boxes, Cpu, FolderOpen, HardDrive, Network, Settings2, ShieldCheck, UserRound, Waypoints } from "lucide-react";

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
  return <div className="pages-shell">
    <header className="sim-app-header">
      <Link to="/" className="sim-brand"><span className="sim-brand-mark"><Waypoints size={17} /></span><span>Network Studio</span></Link>
      <nav className="pages-nav" aria-label="Application pages">
        {sections.map(({ to, label, icon: Icon }) => to === "/appliances"
          ? <Link key={to} to="/appliances" search={{ fromProject }} className={`pages-nav-link${active === label ? " is-active" : ""}`}><Icon size={15} />{label}</Link>
          : <Link key={to} to={to as never} className={`pages-nav-link${active === label ? " is-active" : ""}`}><Icon size={15} />{label}</Link>)}
      </nav>
    </header>
    <main className="pages-main">{children}</main>
  </div>;
}

export function PageTitle({ eyebrow, title, description, actions }: { eyebrow?: string; title: string; description?: string; actions?: ReactNode }) {
  return <div className="pages-title-row"><div>{eyebrow && <div className="pages-eyebrow">{eyebrow}</div>}<h1>{title}</h1>{description && <p>{description}</p>}</div>{actions && <div className="pages-title-actions">{actions}</div>}</div>;
}
