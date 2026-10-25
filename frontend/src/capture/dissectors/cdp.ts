import { field, type LayerResult } from "./types";

export function dissectCdp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 4) return undefined;
  const version = data[offset];
  const ttl = data[offset + 1];
  const checksum = (data[offset + 2] << 8) | data[offset + 3];
  const fields = [
    field("Version", String(version), offset, 1),
    field("TTL", `${ttl} seconds`, offset + 1, 1),
    field("Checksum", `0x${checksum.toString(16).padStart(4, "0")}`, offset + 2, 2),
  ];
  let cursor = offset + 4;
  let device = "";
  while (cursor + 4 <= data.length) {
    const type = (data[cursor] << 8) | data[cursor + 1];
    const length = (data[cursor + 2] << 8) | data[cursor + 3];
    if (length < 4 || cursor + length > data.length) break;
    const value = data.subarray(cursor + 4, cursor + length);
    const names: Record<number, string> = { 1: "Device ID", 2: "Addresses", 3: "Port ID", 4: "Capabilities", 5: "Software Version", 6: "Platform" };
    const label = names[type] ?? `TLV ${type}`;
    const text = new TextDecoder().decode(value).replace(/\0+$/, "");
    fields.push(field(label, text || `${value.length} bytes`, cursor, length));
    if (type === 1) device = text;
    cursor += length;
  }
  const info = device ? `Device ID: ${device}` : "CDP";
  return { layer: { name: "Cisco Discovery Protocol", summary: info, fields }, protocol: "CDP", info };
}
