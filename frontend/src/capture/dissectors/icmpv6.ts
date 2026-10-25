import { field, read16, type LayerResult } from "./types";

const typeNames: Record<number, string> = {
  1: "Destination unreachable",
  2: "Packet too big",
  3: "Time exceeded",
  128: "Echo (ping) request",
  129: "Echo (ping) reply",
  133: "Router solicitation",
  134: "Router advertisement",
  135: "Neighbor solicitation",
  136: "Neighbor advertisement",
};

export function dissectIcmpv6(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 4) return undefined;
  const type = data[offset];
  const code = data[offset + 1];
  const name = typeNames[type] ?? `Type ${type}`;
  const fields = [
    field("Type", `${name} (${type})`, offset, 1),
    field("Code", String(code), offset + 1, 1),
    field("Checksum", `0x${read16(data, offset + 2).toString(16).padStart(4, "0")}`, offset + 2, 2),
  ];
  let info = name;
  if ((type === 128 || type === 129) && data.length >= offset + 8) {
    const id = read16(data, offset + 4);
    const sequence = read16(data, offset + 6);
    fields.push(field("Identifier", `0x${id.toString(16).padStart(4, "0")}`, offset + 4, 2));
    fields.push(field("Sequence Number", String(sequence), offset + 6, 2));
    info += `  id=0x${id.toString(16).padStart(4, "0")}, seq=${sequence}`;
  }
  return {
    layer: { name: "Internet Control Message Protocol v6", summary: info, fields },
    protocol: "ICMPv6",
    info,
  };
}
