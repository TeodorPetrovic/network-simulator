import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { api } from "../api";

export function AuthenticatedSymbolImage({ symbolId, fallback }: { symbolId: string; fallback: ReactNode }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    setUrl(null);
    if (typeof api.symbolRaw !== "function") return;
    void api.symbolRaw(symbolId).then((blob) => {
      objectUrl = URL.createObjectURL(blob);
      if (cancelled) URL.revokeObjectURL(objectUrl);
      else setUrl(objectUrl);
    }).catch(() => {});
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [symbolId]);
  return url
    ? <image className="sim-node-symbol" href={url} x="-32" y="-32" width="64" height="64" preserveAspectRatio="xMidYMid meet" />
    : fallback;
}


// Rewrites the outer <svg> width/height; the viewBox is kept so the content scales with it.
export function resizeSvgMarkup(svg: string, width: number, height: number): string {
  return svg.replace(/^(\s*<svg\b[^>]*?)\swidth="[^"]*"([^>]*?)\sheight="[^"]*"/, `$1 width="${width}"$2 height="${height}"`);
}

