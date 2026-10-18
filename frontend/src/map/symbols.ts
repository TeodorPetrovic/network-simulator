export type CanvasSymbol =
  | { kind: "custom"; symbolId: string }
  | { kind: "default"; name: string }
  | { kind: "type" };

const defaultByNodeType: Record<string, string> = {
  qemu: "router",
  dynamips: "router",
  docker: "pc",
  vpcs: "pc",
  ethernet_switch: "switch",
  ethernet_hub: "switch",
  cloud: "cloud",
  nat: "cloud",
};

function fileName(symbolId: string): string {
  const clean = symbolId.trim().replace(/^:\/?symbols\//, "");
  return clean.split(/[\\/]/).at(-1)?.toLocaleLowerCase() ?? "";
}

export function resolveCanvasSymbol(
  nodeType: string,
  symbolId: unknown,
  defaults: Record<string, string>,
  customIds: string[],
): CanvasSymbol {
  const requested = typeof symbolId === "string" ? symbolId : "";
  const requestedFile = fileName(requested);
  if (requestedFile) {
    const custom = customIds.find((id) => id === requested || fileName(id) === requestedFile);
    if (custom) return { kind: "custom", symbolId: custom };
    const entry = Object.entries(defaults).find(([, file]) => fileName(file) === requestedFile);
    if (entry) return { kind: "default", name: entry[0] };
  }

  const preferredName = defaultByNodeType[nodeType];
  if (preferredName && Object.keys(defaults).some((name) => name.toLocaleLowerCase() === preferredName)) {
    return { kind: "default", name: preferredName };
  }
  return { kind: "type" };
}
