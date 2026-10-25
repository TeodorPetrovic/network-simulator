export type PcapPacket = {
  timestamp: number;
  data: Uint8Array;
  originalLength: number;
  linkType: number;
};

export type PcapHeader = {
  littleEndian: boolean;
  nanoseconds: boolean;
  linkType: number;
  snapLength: number;
};

export class PcapError extends Error {}

const MAX_PACKET_LENGTH = 64 * 1024 * 1024;
const MAX_BLOCK_LENGTH = 128 * 1024 * 1024;

function append(left: Uint8Array, right: Uint8Array): Uint8Array {
  const result = new Uint8Array(left.length + right.length);
  result.set(left);
  result.set(right, left.length);
  return result;
}

function view(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function magic(bytes: Uint8Array): string {
  return Array.from(bytes.subarray(0, 4), (value) => value.toString(16).padStart(2, "0")).join("");
}

function parseClassicHeader(bytes: Uint8Array): PcapHeader {
  const formats: Record<string, { littleEndian: boolean; nanoseconds: boolean }> = {
    d4c3b2a1: { littleEndian: true, nanoseconds: false },
    a1b2c3d4: { littleEndian: false, nanoseconds: false },
    "4d3cb2a1": { littleEndian: true, nanoseconds: true },
    a1b23c4d: { littleEndian: false, nanoseconds: true },
  };
  const format = formats[magic(bytes)];
  if (!format) throw new PcapError("Unrecognized capture format.");

  const data = view(bytes);
  const linkType = data.getUint32(20, format.littleEndian) & 0xffff;
  return {
    ...format,
    linkType,
    snapLength: data.getUint32(16, format.littleEndian),
  };
}

function readOptionResolution(bytes: Uint8Array, littleEndian: boolean): number {
  const data = view(bytes);
  let resolution = 1e-6;
  for (let offset = 0; offset + 4 <= bytes.length;) {
    const code = data.getUint16(offset, littleEndian);
    const length = data.getUint16(offset + 2, littleEndian);
    const valueOffset = offset + 4;
    if (code === 0) break;
    if (code === 9 && length >= 1 && valueOffset < bytes.length) {
      const value = bytes[valueOffset];
      resolution = value & 0x80 ? 2 ** -(value & 0x7f) : 10 ** -value;
    }
    offset = valueOffset + Math.ceil(length / 4) * 4;
  }
  return resolution;
}

/** Read classic pcap or pcapng packets as soon as a complete record arrives. */
export async function* readPcap(
  stream: ReadableStream<Uint8Array>,
): AsyncGenerator<PcapPacket, PcapHeader> {
  const reader = stream.getReader();
  let buffer: Uint8Array<ArrayBufferLike> = new Uint8Array(0);
  let format: "classic" | "pcapng" | undefined;
  let classicHeader: PcapHeader | undefined;
  let sectionLittleEndian = true;
  let interfaces: Array<{ linkType: number; snapLength: number; timestampResolution: number }> = [];
  let sawSection = false;

  try {
    while (true) {
      const next = await reader.read();
      if (!next.done) buffer = append(buffer, next.value);

      if (!format && buffer.length >= 4) {
        format = magic(buffer) === "0a0d0d0a" ? "pcapng" : "classic";
      }

      if (format === "classic") {
        if (!classicHeader && buffer.length >= 24) {
          classicHeader = parseClassicHeader(buffer.subarray(0, 24));
          buffer = buffer.subarray(24);
        }
        while (classicHeader && buffer.length >= 16) {
          const data = view(buffer);
          const includedLength = data.getUint32(8, classicHeader.littleEndian);
          if (includedLength > MAX_PACKET_LENGTH) {
            throw new PcapError("Capture packet exceeds the 64 MiB safety limit.");
          }
          const recordLength = 16 + includedLength;
          if (buffer.length < recordLength) break;
          const seconds = data.getUint32(0, classicHeader.littleEndian);
          const fraction = data.getUint32(4, classicHeader.littleEndian);
          yield {
            timestamp: seconds + fraction / (classicHeader.nanoseconds ? 1e9 : 1e6),
            data: buffer.slice(16, recordLength),
            originalLength: data.getUint32(12, classicHeader.littleEndian),
            linkType: classicHeader.linkType,
          };
          buffer = buffer.subarray(recordLength);
        }
      }

      if (format === "pcapng") {
        while (true) {
          if (!sawSection) {
            if (buffer.length < 12) break;
            const byteOrder = magic(buffer.subarray(8, 12));
            if (byteOrder === "4d3c2b1a") sectionLittleEndian = true;
            else if (byteOrder === "1a2b3c4d") sectionLittleEndian = false;
            else throw new PcapError(`Invalid pcapng byte-order magic: ${byteOrder}.`);
            sawSection = true;
          }
          if (buffer.length < 12) break;

          const blockType = view(buffer).getUint32(0, sectionLittleEndian);
          const blockLength = view(buffer).getUint32(4, sectionLittleEndian);
          if (blockLength < 12 || blockLength > MAX_BLOCK_LENGTH || blockLength % 4 !== 0) {
            throw new PcapError("Invalid pcapng block length.");
          }
          if (buffer.length < blockLength) break;
          const block = buffer.subarray(0, blockLength);
          const blockView = view(block);
          if (blockView.getUint32(blockLength - 4, sectionLittleEndian) !== blockLength) {
            throw new PcapError("Mismatched pcapng block length.");
          }

          if (blockType === 0x0a0d0d0a) {
            const byteOrder = magic(block.subarray(8, 12));
            if (byteOrder !== "4d3c2b1a" && byteOrder !== "1a2b3c4d") {
              throw new PcapError("Invalid pcapng byte-order magic.");
            }
            sectionLittleEndian = byteOrder === "4d3c2b1a";
            interfaces = [];
            sawSection = true;
          } else if (blockType === 1) {
            if (blockLength < 20) throw new PcapError("Invalid pcapng interface description block.");
            const linkType = blockView.getUint16(8, sectionLittleEndian);
            const snapLength = blockView.getUint32(12, sectionLittleEndian);
            const options = block.subarray(16, blockLength - 4);
            interfaces.push({
              linkType,
              snapLength,
              timestampResolution: readOptionResolution(options, sectionLittleEndian),
            });
          } else if (blockType === 6) {
            if (blockLength < 32) throw new PcapError("Invalid pcapng enhanced packet block.");
            const interfaceId = blockView.getUint32(8, sectionLittleEndian);
            const network = interfaces[interfaceId];
            const capturedLength = blockView.getUint32(20, sectionLittleEndian);
            const originalLength = blockView.getUint32(24, sectionLittleEndian);
            if (capturedLength > MAX_PACKET_LENGTH || !network) {
              throw new PcapError("Invalid pcapng packet interface or length.");
            }
            const packetEnd = 28 + capturedLength;
            if (packetEnd > blockLength - 4) throw new PcapError("Truncated pcapng packet data.");
            const ticks = blockView.getUint32(12, sectionLittleEndian) * 2 ** 32 +
              blockView.getUint32(16, sectionLittleEndian);
            yield {
              timestamp: ticks * network.timestampResolution,
              data: block.slice(28, packetEnd),
              originalLength,
              linkType: network.linkType,
            };
          } else if (blockType === 3) {
            if (blockLength < 16 || !interfaces[0]) {
              throw new PcapError("Invalid pcapng simple packet block.");
            }
            const originalLength = blockView.getUint32(8, sectionLittleEndian);
            const capturedLength = Math.min(originalLength, interfaces[0].snapLength || originalLength, blockLength - 16);
            if (capturedLength > MAX_PACKET_LENGTH) {
              throw new PcapError("Capture packet exceeds the 64 MiB safety limit.");
            }
            yield {
              timestamp: 0,
              data: block.slice(12, 12 + capturedLength),
              originalLength,
              linkType: interfaces[0].linkType,
            };
          }

          buffer = buffer.subarray(blockLength);
          if (buffer.length >= 4 && magic(buffer) === "0a0d0d0a") {
            sawSection = false;
          }
        }
      }

      if (next.done) break;
    }

    if (!format) throw new PcapError("Capture ended before its header was complete.");
    if (buffer.length > 0) throw new PcapError("Capture ended in the middle of a packet or block.");
    if (format === "pcapng") {
      return { littleEndian: sectionLittleEndian, nanoseconds: false, linkType: interfaces[0]?.linkType ?? 1, snapLength: 0 };
    }
    if (!classicHeader) throw new PcapError("Capture ended before the pcap header was complete.");
    return classicHeader;
  } finally {
    reader.releaseLock();
  }
}
