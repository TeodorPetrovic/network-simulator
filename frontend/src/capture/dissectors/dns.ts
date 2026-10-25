import { field, read16, type LayerResult } from "./types";

const recordTypes: Record<number, string> = {
  1: "A", 2: "NS", 5: "CNAME", 6: "SOA", 12: "PTR", 15: "MX", 16: "TXT", 28: "AAAA",
};

function readName(data: Uint8Array, start: number): { name: string; next: number } {
  const labels: string[] = [];
  let offset = start;
  let next = start;
  let jumped = false;
  const visited = new Set<number>();
  while (offset < data.length) {
    const size = data[offset];
    if (size === 0) {
      if (!jumped) next = offset + 1;
      break;
    }
    if ((size & 0xc0) === 0xc0) {
      if (offset + 1 >= data.length) break;
      const pointer = ((size & 0x3f) << 8) | data[offset + 1];
      if (!jumped) next = offset + 2;
      if (visited.has(pointer)) break;
      visited.add(pointer);
      offset = pointer;
      jumped = true;
      continue;
    }
    if (offset + size + 1 > data.length) break;
    labels.push(new TextDecoder().decode(data.subarray(offset + 1, offset + size + 1)));
    offset += size + 1;
    if (!jumped) next = offset;
  }
  return { name: labels.join(".") || ".", next };
}

export function dissectDns(data: Uint8Array, offset: number, transport: "UDP" | "TCP" = "UDP"): LayerResult | undefined {
  if (data.length < offset + 12) return undefined;
  const id = read16(data, offset);
  const flags = read16(data, offset + 2);
  const questions = read16(data, offset + 4);
  const answers = read16(data, offset + 6);
  const authorities = read16(data, offset + 8);
  const additionals = read16(data, offset + 10);
  let cursor = offset + 12;
  const fields = [
    field("Transaction ID", `0x${id.toString(16).padStart(4, "0")}`, offset, 2),
    field("Flags", `0x${flags.toString(16).padStart(4, "0")}`, offset + 2, 2),
    field("Questions", String(questions), offset + 4, 2),
    field("Answer RRs", String(answers), offset + 6, 2),
    field("Authority RRs", String(authorities), offset + 8, 2),
    field("Additional RRs", String(additionals), offset + 10, 2),
  ];
  let firstQuestion = "";
  let firstType = "A";
  for (let index = 0; index < questions && cursor < data.length; index += 1) {
    const name = readName(data, cursor);
    cursor = name.next;
    if (cursor + 4 > data.length) break;
    const type = read16(data, cursor);
    const klass = read16(data, cursor + 2);
    if (index === 0) {
      firstQuestion = name.name;
      firstType = recordTypes[type] ?? String(type);
    }
    fields.push(field(`Question ${index + 1}`, `${name.name} ${recordTypes[type] ?? type} class ${klass}`, cursor - (name.next - (cursor - name.next)), 4));
    cursor += 4;
  }
  const recordCount = answers + authorities + additionals;
  for (let index = 0; index < recordCount && cursor < data.length; index += 1) {
    const name = readName(data, cursor);
    cursor = name.next;
    if (cursor + 10 > data.length) break;
    const type = read16(data, cursor);
    const klass = read16(data, cursor + 2);
    const ttl = ((read16(data, cursor + 4) << 16) | read16(data, cursor + 6)) >>> 0;
    const length = read16(data, cursor + 8);
    const valueOffset = cursor + 10;
    if (valueOffset + length > data.length) break;
    let value = `${length} bytes`;
    if (type === 1 && length === 4) value = Array.from(data.subarray(valueOffset, valueOffset + 4)).join(".");
    else if (type === 28 && length === 16) value = Array.from({ length: 8 }, (_, word) => read16(data, valueOffset + word * 2).toString(16)).join(":");
    else if ([2, 5, 12].includes(type)) value = readName(data, valueOffset).name;
    else if (type === 15 && length >= 3) value = `${read16(data, valueOffset)} ${readName(data, valueOffset + 2).name}`;
    else if (type === 16) value = new TextDecoder().decode(data.subarray(valueOffset + 1, valueOffset + Math.min(length, data[valueOffset] + 1)));
    fields.push(field(`${recordTypes[type] ?? `Type ${type}`} ${name.name}`, `${value} TTL ${ttl} class ${klass}`, cursor, 10 + length));
    cursor = valueOffset + length;
  }
  const response = Boolean(flags & 0x8000);
  const info = `${response ? "Standard query response" : "Standard query"} 0x${id.toString(16).padStart(4, "0")}${firstQuestion ? ` ${firstType} ${firstQuestion}` : ""}`;
  return {
    layer: { name: "Domain Name System", summary: info, fields },
    protocol: "DNS",
    info,
    nextOffset: transport === "TCP" ? offset : undefined,
  };
}
