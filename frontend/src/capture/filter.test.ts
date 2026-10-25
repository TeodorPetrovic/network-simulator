import { describe, expect, it } from "vitest";
import { dissect } from "./dissect";
import { compileDisplayFilter, DisplayFilterError, type PacketRecord } from "./filter";

const bytes = new Uint8Array([
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 0,
  0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2,
  0xc0, 0, 0, 80, 0, 0, 0, 0, 0, 0, 0, 0, 0x50, 2, 0xfa, 0xf0, 0, 0, 0, 0,
]);
const packet: PacketRecord = {
  number: 1,
  time: 0,
  timestamp: 0,
  data: bytes,
  originalLength: bytes.length,
  linkType: 1,
  decoded: dissect(bytes),
};

describe("display filters", () => {
  it("evaluates protocol, field, and boolean expressions", () => {
    expect(packet.decoded).toMatchObject({ protocol: "TCP", src: "10.0.0.1" });
    expect(compileDisplayFilter("tcp")(packet)).toBe(true);
    expect(compileDisplayFilter("ip.src == 10.0.0.1")(packet)).toBe(true);
    expect(compileDisplayFilter("tcp.port == 80")(packet)).toBe(true);
    expect(compileDisplayFilter("tcp.dstport >= 80 && ip.addr == 10.0.0.0/24")(packet)).toBe(true);
    expect(compileDisplayFilter("tcp.flags.syn == 1 and eth.src contains 06:07")(packet)).toBe(true);
    expect(compileDisplayFilter("tcp.flags.ack == 0 || udp")(packet)).toBe(true);
    expect(compileDisplayFilter("!(udp or arp)")(packet)).toBe(true);
    expect(compileDisplayFilter("frame.len > 40")(packet)).toBe(true);
    expect(compileDisplayFilter("ip.src matches ^10\\.0\\.0")(packet)).toBe(true);
  });

  it("rejects malformed expressions and invalid regular expressions", () => {
    expect(() => compileDisplayFilter("tcp && (")).toThrow(DisplayFilterError);
    expect(() => compileDisplayFilter("ip.src matches [")(packet)).toThrow(DisplayFilterError);
  });
});
