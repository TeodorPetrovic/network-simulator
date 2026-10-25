import { field, formatIpv4, read16, type LayerResult } from "./types";

const packetTypes: Record<number, string> = {
  1: "Hello", 2: "Database Description", 3: "Link State Request", 4: "Link State Update", 5: "Link State Acknowledgment",
};

export function dissectOspf(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 24) return undefined;
  const type = data[offset + 1];
  const length = read16(data, offset + 2);
  const routerId = formatIpv4(data, offset + 4);
  const areaId = formatIpv4(data, offset + 8);
  const info = `${packetTypes[type] ?? `Type ${type}`} Packet`;
  return {
    layer: {
      name: "Open Shortest Path First",
      summary: info,
      fields: [
        field("Version", String(data[offset]), offset, 1),
        field("Packet Type", info, offset + 1, 1),
        field("Packet Length", String(length), offset + 2, 2),
        field("Router ID", routerId, offset + 4, 4),
        field("Area ID", areaId, offset + 8, 4),
        field("Checksum", `0x${read16(data, offset + 12).toString(16).padStart(4, "0")}`, offset + 12, 2),
        field("Authentication Type", String(read16(data, offset + 14)), offset + 14, 2),
        field("Authentication", Array.from(data.subarray(offset + 16, offset + 24), (v) => v.toString(16).padStart(2, "0")).join(""), offset + 16, 8),
      ],
    },
    protocol: "OSPF",
    info,
  };
}
