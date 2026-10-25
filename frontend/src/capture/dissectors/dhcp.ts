import { field, formatIpv4, formatMac, read32, type LayerResult } from "./types";

const messageTypes: Record<number, string> = {
  1: "Discover", 2: "Offer", 3: "Request", 4: "Decline", 5: "ACK", 6: "NAK", 7: "Release", 8: "Inform",
};

export function dissectDhcp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 240) return undefined;
  const fields = [
    field("Message type", data[offset] === 1 ? "Boot Request" : "Boot Reply", offset, 1),
    field("Hardware type", String(data[offset + 1]), offset + 1, 1),
    field("Hardware address length", String(data[offset + 2]), offset + 2, 1),
    field("Hops", String(data[offset + 3]), offset + 3, 1),
    field("Transaction ID", `0x${read32(data, offset + 4).toString(16).padStart(8, "0")}`, offset + 4, 4),
    field("Seconds elapsed", String((data[offset + 8] << 8) | data[offset + 9]), offset + 8, 2),
    field("Client IP address", formatIpv4(data, offset + 12), offset + 12, 4),
    field("Your IP address", formatIpv4(data, offset + 16), offset + 16, 4),
    field("Server IP address", formatIpv4(data, offset + 20), offset + 20, 4),
    field("Client MAC address", formatMac(data, offset + 28), offset + 28, 6),
  ];
  let messageType = "Unknown";
  let cursor = offset + 240;
  while (cursor < data.length) {
    const code = data[cursor++];
    if (code === 255) break;
    if (code === 0) continue;
    if (cursor >= data.length) break;
    const length = data[cursor++];
    if (cursor + length > data.length) break;
    const valueOffset = cursor;
    let value = Array.from(data.subarray(cursor, cursor + length), (byte) => byte.toString(16).padStart(2, "0")).join(":");
    if (code === 53 && length === 1) {
      messageType = messageTypes[data[cursor]] ?? `Unknown (${data[cursor]})`;
      value = messageType;
    } else if ([50, 54].includes(code) && length === 4) {
      value = formatIpv4(data, cursor);
    } else if ([12, 15].includes(code)) {
      value = new TextDecoder().decode(data.subarray(cursor, cursor + length));
    }
    fields.push(field(`Option ${code}`, value, valueOffset, length));
    cursor += length;
  }
  const info = `DHCP ${messageType}`;
  return { layer: { name: "Dynamic Host Configuration Protocol", summary: info, fields }, protocol: "DHCP", info };
}
