import { field, formatIpv4, read16, type LayerResult } from "./types";

export function dissectRip(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 4) return undefined;
  const command = data[offset];
  const info = command === 1 ? "Request" : command === 2 ? "Response" : `Command ${command}`;
  const fields = [
    field("Command", info, offset, 1),
    field("Version", String(data[offset + 1]), offset + 1, 1),
    field("Routing Domain", String(read16(data, offset + 2)), offset + 2, 2),
  ];
  const entrySize = data[offset + 1] === 2 ? 20 : 20;
  for (let cursor = offset + 4, index = 1; cursor + entrySize <= data.length; cursor += entrySize, index += 1) {
    const address = formatIpv4(data, cursor + 4);
    const metric = ((data[cursor + 16] << 24) | (data[cursor + 17] << 16) | (data[cursor + 18] << 8) | data[cursor + 19]) >>> 0;
    fields.push(field(`Route ${index}`, `${address}, metric ${metric}`, cursor, entrySize));
  }
  return { layer: { name: "Routing Information Protocol", summary: info, fields }, protocol: "RIP", info };
}
