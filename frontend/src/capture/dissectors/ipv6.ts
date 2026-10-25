import { field, formatIpv6, read16, type LayerResult } from "./types";

export function dissectIpv6(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 40 || data[offset] >> 4 !== 6) return undefined;
  const trafficClass = ((data[offset] & 0x0f) << 4) | (data[offset + 1] >> 4);
  const flowLabel = ((data[offset + 1] & 0x0f) << 16) | (data[offset + 2] << 8) | data[offset + 3];
  const payloadLength = read16(data, offset + 4);
  const nextHeader = data[offset + 6];
  const hopLimit = data[offset + 7];
  const source = formatIpv6(data, offset + 8);
  const destination = formatIpv6(data, offset + 24);
  return {
    layer: {
      name: "Internet Protocol Version 6",
      summary: `${source} → ${destination}`,
      fields: [
        field("Version", "6", offset, 1),
        field("Traffic Class", `0x${trafficClass.toString(16).padStart(2, "0")}`, offset, 2),
        field("Flow Label", `0x${flowLabel.toString(16).padStart(5, "0")}`, offset + 1, 3),
        field("Payload Length", String(payloadLength), offset + 4, 2),
        field("Next Header", String(nextHeader), offset + 6, 1),
        field("Hop Limit", String(hopLimit), offset + 7, 1),
        field("Source Address", source, offset + 8, 16),
        field("Destination Address", destination, offset + 24, 16),
      ],
    },
    protocol: "IPv6",
    info: "",
    source,
    destination,
    nextOffset: offset + 40,
    nextProtocol: nextHeader,
  };
}
