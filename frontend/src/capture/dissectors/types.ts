export type Field = {
  label: string;
  value: string;
  byteOffset: number;
  length: number;
};

export type Layer = {
  name: string;
  summary: string;
  fields: Field[];
  children?: Layer[];
};

export type LayerResult = {
  layer: Layer;
  protocol: string;
  info: string;
  source?: string;
  destination?: string;
  nextOffset?: number;
  nextProtocol?: number;
};

export const field = (label: string, value: string, byteOffset: number, length: number): Field => ({
  label,
  value,
  byteOffset,
  length,
});

export const read16 = (data: Uint8Array, offset: number): number =>
  ((data[offset] ?? 0) << 8) | (data[offset + 1] ?? 0);

export const read32 = (data: Uint8Array, offset: number): number =>
  (((read16(data, offset) << 16) | read16(data, offset + 2)) >>> 0);

export const formatMac = (data: Uint8Array, offset: number): string =>
  Array.from(data.subarray(offset, offset + 6), (value) => value.toString(16).padStart(2, "0")).join(":");

export const formatIpv4 = (data: Uint8Array, offset: number): string =>
  Array.from(data.subarray(offset, offset + 4)).join(".");

export const formatIpv6 = (data: Uint8Array, offset: number): string => {
  const words = Array.from({ length: 8 }, (_, index) => read16(data, offset + index * 2).toString(16));
  return words.join(":");
};
