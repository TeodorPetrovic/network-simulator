import { field, read16, read32, type LayerResult } from "./types";

export function dissectTcp(data: Uint8Array, offset: number, ttl?: number): LayerResult | undefined {
  if (data.length < offset + 20) return undefined;
  const sourcePort = read16(data, offset);
  const destinationPort = read16(data, offset + 2);
  const sequence = read32(data, offset + 4);
  const acknowledgment = read32(data, offset + 8);
  const headerLength = (data[offset + 12] >> 4) * 4;
  if (headerLength < 20 || data.length < offset + headerLength) return undefined;
  const flagByte = data[offset + 13];
  const flagNames = [
    [0x80, "CWR"], [0x40, "ECE"], [0x20, "URG"], [0x10, "ACK"],
    [0x08, "PSH"], [0x04, "RST"], [0x02, "SYN"], [0x01, "FIN"],
  ] as const;
  const flags = flagNames.filter(([mask]) => flagByte & mask).map(([, name]) => name);
  const payloadLength = Math.max(0, data.length - offset - headerLength);
  const source = String(sourcePort);
  const destination = String(destinationPort);
  const info = `${sourcePort} → ${destinationPort} [${flags.join(", ")}] Seq=${sequence}${flagByte & 0x10 ? ` Ack=${acknowledgment}` : ""} Win=${read16(data, offset + 14)} Len=${payloadLength}`;
  const fields = [
    field("Source Port", source, offset, 2),
    field("Destination Port", destination, offset + 2, 2),
    field("Sequence Number", String(sequence), offset + 4, 4),
    field("Acknowledgment Number", String(acknowledgment), offset + 8, 4),
    field("Header Length", `${headerLength} bytes`, offset + 12, 1),
    ...flagNames.map(([mask, name]) => field(name, flagByte & mask ? "Set" : "Not set", offset + 13, 1)),
    field("Window", String(read16(data, offset + 14)), offset + 14, 2),
    field("Checksum", `0x${read16(data, offset + 16).toString(16).padStart(4, "0")}`, offset + 16, 2),
    field("Urgent pointer", String(read16(data, offset + 18)), offset + 18, 2),
  ];

  for (let optionOffset = offset + 20; optionOffset < offset + headerLength;) {
    const kind = data[optionOffset];
    if (kind === 0) {
      fields.push(field("Option", "End of options", optionOffset, 1));
      break;
    }
    if (kind === 1) {
      fields.push(field("Option", "No operation (NOP)", optionOffset, 1));
      optionOffset += 1;
      continue;
    }
    if (optionOffset + 2 > offset + headerLength) break;
    const length = data[optionOffset + 1];
    if (length < 2 || optionOffset + length > offset + headerLength) break;
    const optionName = data[kind === 2 ? optionOffset + 2 : optionOffset] === undefined
      ? `TCP option ${kind}`
      : ({ 2: "Maximum segment size", 3: "Window scale", 4: "SACK permitted", 5: "SACK", 8: "Timestamps" } as Record<number, string>)[kind] ?? `TCP option ${kind}`;
    const value = kind === 2 && length >= 4
      ? String(read16(data, optionOffset + 2))
      : kind === 3 && length >= 3
        ? String(data[optionOffset + 2])
        : kind === 8 && length >= 10
          ? `${read32(data, optionOffset + 2)} / ${read32(data, optionOffset + 6)}`
          : `${length} bytes`;
    fields.push(field(optionName, value, optionOffset, length));
    optionOffset += length;
  }

  return {
    layer: { name: "Transmission Control Protocol", summary: info, fields },
    protocol: "TCP",
    info: ttl === undefined ? info : `${info}, ttl=${ttl}`,
    source,
    destination,
    nextOffset: offset + headerLength,
  };
}
