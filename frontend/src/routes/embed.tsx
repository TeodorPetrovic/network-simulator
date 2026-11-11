import { createFileRoute, useSearch } from "@tanstack/react-router";
import { useEffect } from "react";

export const Route = createFileRoute("/embed")({
  validateSearch: (search: Record<string, unknown>) => ({
    url: typeof search.url === "string" ? search.url : "",
    title: typeof search.title === "string" ? search.title : "Network Studio",
  }),
  component: EmbedRoute,
});

/** A pop-out window that frames a controller-hosted page (for example the Web Wireshark client). */
function EmbedRoute() {
  const { url, title } = useSearch({ from: "/embed" });
  useEffect(() => { document.title = title; }, [title]);
  if (!/^https?:\/\//i.test(url)) return <main className="p-6 text-sm">Nothing to show.</main>;
  return <iframe title={title} src={url} style={{ position: "fixed", inset: 0, width: "100%", height: "100%", border: 0 }} />;
}
