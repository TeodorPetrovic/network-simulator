import { field, read16, type LayerResult } from "./types";

export function dissectUdp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 8) return undefined;
  const sourcePort = read16(data, offset);
  const destinationPort = read16(data, offset + 2);
  const length = read16(data, offset + 4);
  const info = `${sourcePort} → ${destinationPort} Len=${length}`;
  return {
    layer: {
      name: "User Datagram Protocol",
      summary: info,
      fields: [
        field("Source Port", String(sourcePort), offset, 2),
        field("Destination Port", String(destinationPort), offset + 2, 2),
        field("Length", String(length), offset + 4, 2),
        field("Checksum", `0x${read16(data, offset + 6).toString(16).padStart(4, "0")}`, offset + 6, 2),
      ],
    },
    protocol: "UDP",
    info,
    source: String(sourcePort),
    destination: String(destinationPort),
    nextOffset: offset + 8,
  };
}
