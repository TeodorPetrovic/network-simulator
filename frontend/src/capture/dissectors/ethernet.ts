import { field, formatMac, read16, type LayerResult } from "./types";

export function dissectEthernet(data: Uint8Array, offset = 0): LayerResult | undefined {
  if (data.length < offset + 14) return undefined;
  const destination = formatMac(data, offset);
  const source = formatMac(data, offset + 6);
  const etherType = read16(data, offset + 12);
  return {
    layer: {
      name: "Ethernet II",
      summary: `${source} → ${destination}`,
      fields: [
        field("Destination", destination, offset, 6),
        field("Source", source, offset + 6, 6),
        field("Type", `0x${etherType.toString(16).padStart(4, "0")}`, offset + 12, 2),
      ],
    },
    protocol: "Ethernet",
    info: "",
    source,
    destination,
    nextOffset: offset + 14,
    nextProtocol: etherType,
  };
}
