import { field, formatIpv4, read16, type LayerResult } from "./types";

export function dissectIpv4(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 20 || data[offset] >> 4 !== 4) return undefined;
  const version = data[offset] >> 4;
  const headerLength = (data[offset] & 0x0f) * 4;
  if (headerLength < 20 || data.length < offset + headerLength) return undefined;
  const dscpEcn = data[offset + 1];
  const totalLength = read16(data, offset + 2);
  const identification = read16(data, offset + 4);
  const fragment = read16(data, offset + 6);
  const ttl = data[offset + 8];
  const protocol = data[offset + 9];
  const source = formatIpv4(data, offset + 12);
  const destination = formatIpv4(data, offset + 16);
  const flags = fragment >> 13;
  const fragmentOffset = fragment & 0x1fff;
  const fields = [
    field("Version", String(version), offset, 1),
    field("Header Length", `${headerLength} bytes`, offset, 1),
    field("Differentiated Services Field", `0x${dscpEcn.toString(16).padStart(2, "0")}`, offset + 1, 1),
    field("DSCP", String(dscpEcn >> 2), offset + 1, 1),
    field("ECN", String(dscpEcn & 3), offset + 1, 1),
    field("Total Length", String(totalLength), offset + 2, 2),
    field("Identification", `0x${identification.toString(16).padStart(4, "0")}`, offset + 4, 2),
    field("Flags", `0x${flags.toString(16)}`, offset + 6, 2),
    field("Reserved bit", String((flags >> 2) & 1), offset + 6, 2),
    field("Don't fragment", String((flags >> 1) & 1), offset + 6, 2),
    field("More fragments", String(flags & 1), offset + 6, 2),
    field("Fragment offset", `${fragmentOffset} (${fragmentOffset * 8} bytes)`, offset + 6, 2),
    field("Time to live", String(ttl), offset + 8, 1),
    field("Protocol", `${protocol}`, offset + 9, 1),
    field("Header checksum", `0x${read16(data, offset + 10).toString(16).padStart(4, "0")}`, offset + 10, 2),
    field("Source Address", source, offset + 12, 4),
    field("Destination Address", destination, offset + 16, 4),
  ];
  return {
    layer: { name: "Internet Protocol Version 4", summary: `${source} → ${destination}`, fields },
    protocol: "IPv4",
    info: "",
    source,
    destination,
    nextOffset: offset + headerLength,
    nextProtocol: protocol,
  };
}
