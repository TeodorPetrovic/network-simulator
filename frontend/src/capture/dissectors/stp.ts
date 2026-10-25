import { field, formatMac, type LayerResult } from "./types";

export function dissectStp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 4) return undefined;
  const protocolId = (data[offset] << 8) | data[offset + 1];
  const version = data[offset + 2];
  const type = data[offset + 3];
  const info = type === 0 ? "Configuration" : type === 0x80 ? "Topology Change Notification" : `BPDU type ${type}`;
  const fields = [
    field("Protocol Identifier", String(protocolId), offset, 2),
    field("Protocol Version Identifier", String(version), offset + 2, 1),
    field("BPDU Type", info, offset + 3, 1),
  ];
  if (data.length >= offset + 35 && type === 0) {
    fields.push(field("Root Identifier", Array.from(data.subarray(offset + 5, offset + 13), (v) => v.toString(16).padStart(2, "0")).join(":"), offset + 5, 8));
    fields.push(field("Root Path Cost", String((data[offset + 13] * 2 ** 24) + (data[offset + 14] << 16) + (data[offset + 15] << 8) + data[offset + 16]), offset + 13, 4));
    fields.push(field("Bridge Identifier", Array.from(data.subarray(offset + 17, offset + 25), (v) => v.toString(16).padStart(2, "0")).join(":"), offset + 17, 8));
    fields.push(field("Port Identifier", `0x${((data[offset + 25] << 8) | data[offset + 26]).toString(16)}`, offset + 25, 2));
    fields.push(field("Message Age", String(data[offset + 27]), offset + 27, 2));
  }
  void formatMac;
  return { layer: { name: "Spanning Tree Protocol", summary: info, fields }, protocol: "STP", info };
}
