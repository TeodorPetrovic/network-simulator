import { useEffect, useState, type ReactNode } from "react";
import { needsServer } from "../api";
import {
  createRootRoute,
  useNavigate,
  HeadContent,
  Outlet,
  Scripts,
} from "@tanstack/react-router";
import { TitleTooltips } from "../TitleTooltips";
import { themeBootScript, watchSystemTheme } from "../theme";
import appCss from "../styles.css?url";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { name: "theme-color", content: "#f5f7fb" },
      { name: "description", content: "A fast, local-first network lab workspace." },
      { title: "Network Studio" },
    ],
    links: [{ rel: "stylesheet", href: appCss }],
  }),
  component: RootComponent,
});

function RootComponent() {
  const [clientReady, setClientReady] = useState(false);
  const navigate = useNavigate();
  useEffect(() => watchSystemTheme(), []);
  useEffect(() => {
    // The desktop app has no same-origin controller: ask for one before anything else loads.
    if (needsServer() && window.location.pathname !== "/connect") void navigate({ to: "/connect", replace: true });
    setClientReady(true);
  }, [navigate]);

  return (
    <RootDocument>
      {clientReady && <Outlet />}
      <TitleTooltips />
    </RootDocument>
  );
}

function RootDocument({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
        <script dangerouslySetInnerHTML={{ __html: themeBootScript }} />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}
