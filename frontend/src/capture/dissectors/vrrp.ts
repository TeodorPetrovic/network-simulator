import { field, formatIpv4, type LayerResult } from "./types";

export function dissectVrrp(data: Uint8Array, offset: number): LayerResult | undefined {
  if (data.length < offset + 8) return undefined;
  const versionType = data[offset];
  const version = versionType >> 4;
  const type = versionType & 0x0f;
  const vrid = data[offset + 1];
  const priority = data[offset + 2];
  const count = data[offset + 3];
  const fields = [
    field("Version", String(version), offset, 1),
    field("Type", type === 1 ? "Advertisement" : String(type), offset, 1),
    field("Virtual Router ID", String(vrid), offset + 1, 1),
    field("Priority", String(priority), offset + 2, 1),
    field("Address Count", String(count), offset + 3, 1),
    field("Authentication Type", String(data[offset + 4]), offset + 4, 1),
    field("Advertisement Interval", `${data[offset + 5]} seconds`, offset + 5, 1),
    field("Checksum", `0x${((data[offset + 6] << 8) | data[offset + 7]).toString(16).padStart(4, "0")}`, offset + 6, 2),
  ];
  for (let index = 0; index < count && offset + 8 + (index + 1) * 4 <= data.length; index += 1) {
    fields.push(field(`IP Address ${index + 1}`, formatIpv4(data, offset + 8 + index * 4), offset + 8 + index * 4, 4));
  }
  const info = `Advertisement, vrid ${vrid}, prio ${priority}, intvl ${data[offset + 5]}s`;
  return { layer: { name: "Virtual Router Redundancy Protocol", summary: info, fields }, protocol: "VRRP", info };
}
