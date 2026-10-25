import { createFileRoute, useSearch } from "@tanstack/react-router";
import { CaptureViewer } from "../capture/CaptureViewer";

export const Route = createFileRoute("/capture")({
  validateSearch: (search: Record<string, unknown>) => ({
    stream: typeof search.stream === "string" ? search.stream : undefined,
    download: typeof search.download === "string" ? search.download : undefined,
    title: typeof search.title === "string" ? search.title : undefined,
  }),
  component: CaptureRoute,
});

function CaptureRoute() {
  const { stream, download, title } = useSearch({ from: "/capture" }) as {
    stream?: string;
    download?: string;
    title?: string;
  };
  return <main style={{ position: "fixed", inset: 0 }}>
    <CaptureViewer streamUrl={stream} downloadUrl={download} title={title ?? "Capture Viewer"} />
  </main>;
}
