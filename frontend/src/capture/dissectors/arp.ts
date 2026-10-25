import { field, formatIpv4, formatMac, read16, type LayerResult } from "./types";

export function dissectArp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 28) return undefined;
  const operation = read16(data, offset + 6);
  const senderMac = formatMac(data, offset + 8);
  const senderIp = formatIpv4(data, offset + 14);
  const targetMac = formatMac(data, offset + 18);
  const targetIp = formatIpv4(data, offset + 24);
  const request = operation === 1;
  const info = request
    ? `Who has ${targetIp}? Tell ${senderIp}`
    : operation === 2
      ? `${senderIp} is at ${senderMac}`
      : `ARP (opcode ${operation})`;
  return {
    layer: {
      name: "Address Resolution Protocol",
      summary: info,
      fields: [
        field("Hardware type", String(read16(data, offset)), offset, 2),
        field("Protocol type", `0x${read16(data, offset + 2).toString(16)}`, offset + 2, 2),
        field("Hardware size", String(data[offset + 4]), offset + 4, 1),
        field("Protocol size", String(data[offset + 5]), offset + 5, 1),
        field("Opcode", request ? "request (1)" : operation === 2 ? "reply (2)" : String(operation), offset + 6, 2),
        field("Sender MAC address", senderMac, offset + 8, 6),
        field("Sender IP address", senderIp, offset + 14, 4),
        field("Target MAC address", targetMac, offset + 18, 6),
        field("Target IP address", targetIp, offset + 24, 4),
      ],
    },
    protocol: "ARP",
    info,
  };
}
