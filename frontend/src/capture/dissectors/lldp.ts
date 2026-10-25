import { field, formatMac, type LayerResult } from "./types";

export function dissectLldp(data: Uint8Array, offset: number): LayerResult | undefined {
  const fields = [];
  let cursor = offset;
  let info = "";
  while (cursor + 2 <= data.length) {
    const header = (data[cursor] << 8) | data[cursor + 1];
    const type = header >> 9;
    const length = header & 0x1ff;
    const valueOffset = cursor + 2;
    if (valueOffset + length > data.length) break;
    let label = `TLV ${type}`;
    let value = `${length} bytes`;
    if (type === 0) { label = "End of LLDPDU"; value = ""; }
    if (type === 1 && length >= 7) { label = "Chassis ID"; value = `${data[valueOffset]}: ${formatMac(data, valueOffset + 1)}`; }
    if (type === 2 && length >= 2) { label = "Port ID"; value = `${data[valueOffset]}: ${Array.from(data.subarray(valueOffset + 1, valueOffset + length)).map((v) => String.fromCharCode(v)).join("")}`; }
    if (type === 3 && length === 2) { label = "Time to Live"; value = String((data[valueOffset] << 8) | data[valueOffset + 1]); }
    if (type === 5 || type === 6 || type === 7) { label = type === 5 ? "System Name" : type === 6 ? "System Description" : "System Capabilities"; value = new TextDecoder().decode(data.subarray(valueOffset, valueOffset + length)); }
    fields.push(field(label, value, cursor, length + 2));
    if (type === 0) break;
    cursor = valueOffset + length;
  }
  info = fields.find((entry) => entry.label === "System Name")?.value || "LLDP";
  return { layer: { name: "Link Layer Discovery Protocol", summary: info, fields }, protocol: "LLDP", info };
}
