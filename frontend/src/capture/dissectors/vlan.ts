import { field, read16, type LayerResult } from "./types";

export function dissectVlan(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 4) return undefined;
  const tag = read16(data, offset);
  const priority = tag >> 13;
  const dei = (tag >> 12) & 1;
  const id = tag & 0x0fff;
  const etherType = read16(data, offset + 2);
  return {
    layer: {
      name: "802.1Q Virtual LAN",
      summary: `ID: ${id}, Priority: ${priority}`,
      fields: [
        field("Priority", String(priority), offset, 2),
        field("Drop eligible indicator", String(dei), offset, 2),
        field("VLAN ID", String(id), offset, 2),
        field("Type", `0x${etherType.toString(16).padStart(4, "0")}`, offset + 2, 2),
      ],
    },
    protocol: "VLAN",
    info: "",
    nextOffset: offset + 4,
    nextProtocol: etherType,
  };
}
