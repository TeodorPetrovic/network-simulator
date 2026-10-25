import { field, read16, type LayerResult } from "./types";

const typeNames: Record<number, string> = {
  0: "Echo (ping) reply",
  3: "Destination unreachable",
  5: "Redirect",
  8: "Echo (ping) request",
  11: "Time-to-live exceeded",
  12: "Parameter problem",
};

export function dissectIcmp(data: Uint8Array, offset: number, ttl?: number): LayerResult | undefined {
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
  if ((type === 0 || type === 8) && data.length >= offset + 8) {
    const id = read16(data, offset + 4);
    const sequence = read16(data, offset + 6);
    fields.push(field("Identifier", `0x${id.toString(16).padStart(4, "0")}`, offset + 4, 2));
    fields.push(field("Sequence Number", String(sequence), offset + 6, 2));
    const payloadLength = Math.max(0, data.length - offset - 8);
    info += `  id=0x${id.toString(16).padStart(4, "0")}, seq=${sequence}/${payloadLength}`;
  }
  if (ttl !== undefined) info += `, ttl=${ttl}`;
  return {
    layer: { name: "Internet Control Message Protocol", summary: info, fields },
    protocol: "ICMP",
    info,
  };
}
