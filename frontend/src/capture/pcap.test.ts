import { describe, expect, it } from "vitest";
import { PcapError, readPcap, type PcapPacket } from "./pcap";

function classicCapture(littleEndian: boolean, nanoseconds = false): Uint8Array {
  const bytes = new Uint8Array(43);
  const data = new DataView(bytes.buffer);
  const magic = littleEndian
    ? nanoseconds ? [0x4d, 0x3c, 0xb2, 0xa1] : [0xd4, 0xc3, 0xb2, 0xa1]
    : nanoseconds ? [0xa1, 0xb2, 0x3c, 0x4d] : [0xa1, 0xb2, 0xc3, 0xd4];
  bytes.set(magic);
  data.setUint16(4, 2, littleEndian);
  data.setUint16(6, 4, littleEndian);
  data.setUint32(16, 65535, littleEndian);
  data.setUint32(20, 1, littleEndian);
  data.setUint32(24, 7, littleEndian);
  data.setUint32(28, nanoseconds ? 500 : 500000, littleEndian);
  data.setUint32(32, 3, littleEndian);
  data.setUint32(36, 3, littleEndian);
  bytes.set([1, 2, 3], 40);
  return bytes;
}

function pcapngCapture(): Uint8Array {
  const bytes = new Uint8Array(100);
  const data = new DataView(bytes.buffer);
  let offset = 0;
  const block = (type: number, length: number, write: (view: DataView) => void) => {
    data.setUint32(offset, type, true);
    data.setUint32(offset + 4, length, true);
    write(new DataView(bytes.buffer, offset, length));
    data.setUint32(offset + length - 4, length, true);
    offset += length;
  };

  block(0x0a0d0d0a, 28, (view) => {
    view.setUint32(8, 0x1a2b3c4d, true);
    view.setUint16(12, 1, true);
    view.setUint16(14, 0, true);
    view.setUint32(16, 0xffffffff, true);
  });
  block(1, 32, (view) => {
    view.setUint16(8, 1, true);
    view.setUint32(12, 65535, true);
    view.setUint16(16, 9, true);
    view.setUint16(18, 1, true);
    view.setUint8(20, 9);
    view.setUint16(24, 0, true);
    view.setUint16(26, 0, true);
  });
  block(6, 36, (view) => {
    view.setUint32(8, 0, true);
    view.setUint32(12, 0, true);
    view.setUint32(16, 2, true);
    view.setUint32(20, 3, true);
    view.setUint32(24, 3, true);
    bytes.set([4, 5, 6], offset + 28);
  });
  return bytes.slice(0, offset);
}

function stream(chunks: Uint8Array[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      chunks.forEach((chunk) => controller.enqueue(chunk));
      controller.close();
    },
  });
}

describe("capture reader", () => {
  it("reads partial classic pcap chunks in either byte order", async () => {
    for (const littleEndian of [true, false]) {
      const bytes = classicCapture(littleEndian);
      const packets = readPcap(stream([bytes.slice(0, 9), bytes.slice(9, 31), bytes.slice(31)]));
      expect(await packets.next()).toMatchObject({
        value: { timestamp: 7.5, originalLength: 3, linkType: 1, data: new Uint8Array([1, 2, 3]) },
      });
      expect((await packets.next()).done).toBe(true);
    }
  });

  it("supports nanosecond classic timestamps", async () => {
    const packets = readPcap(stream([classicCapture(true, true)]));
    expect(((await packets.next()).value as PcapPacket).timestamp).toBe(7.0000005);
  });

  it("reads pcapng sections, interfaces, timestamp resolution, and enhanced packets incrementally", async () => {
    const bytes = pcapngCapture();
    const packets = readPcap(stream([bytes.slice(0, 15), bytes.slice(15, 56), bytes.slice(56)]));
    expect(await packets.next()).toMatchObject({
      value: { timestamp: 2e-9, linkType: 1, data: new Uint8Array([4, 5, 6]) },
    });
    expect((await packets.next()).done).toBe(true);
  });

  it("reports malformed and truncated captures", async () => {
    await expect(readPcap(stream([new Uint8Array([0, 1, 2, 3])])).next()).rejects.toThrow(PcapError);
    await expect(readPcap(stream([classicCapture(true).slice(0, 31)])).next()).rejects.toThrow(PcapError);
  });
});
