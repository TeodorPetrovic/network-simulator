import { describe, expect, it } from "vitest";
import { dissect } from "./dissect";

function ethernetIpv4(protocol: number, payload: Uint8Array): Uint8Array {
  const packet = new Uint8Array(14 + 20 + payload.length);
  packet.set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 0]);
  packet.set([
    0x45, 0, 0, (20 + payload.length) & 0xff, 0, 1, 0, 0,
    64, protocol, 0x12, 0x34, 192, 168, 1, 1, 192, 168, 1, 2,
  ], 14);
  packet.set(payload, 34);
  return packet;
}

function tcpHeader(sourcePort: number, destinationPort: number, flags: number, payload: Uint8Array = new Uint8Array()): Uint8Array {
  const bytes = new Uint8Array(20 + payload.length);
  const data = new DataView(bytes.buffer);
  data.setUint16(0, sourcePort);
  data.setUint16(2, destinationPort);
  data.setUint32(4, 0);
  data.setUint32(8, 0);
  bytes[12] = 0x50;
  bytes[13] = flags;
  data.setUint16(14, 64240);
  data.setUint16(16, 0x1234);
  bytes.set(payload, 20);
  return bytes;
}

describe("packet dissection", () => {
  it("decodes Ethernet and IPv4 TCP fields", () => {
    const packet = ethernetIpv4(6, tcpHeader(49152, 80, 0x02));
    const decoded = dissect(packet);
    expect(decoded.protocol).toBe("TCP");
    expect(decoded.src).toBe("192.168.1.1");
    expect(decoded.info).toContain("49152 → 80 [SYN] Seq=0 Win=64240 Len=0");
    expect(decoded.layers[1].fields.map((item) => item.label)).toEqual([
      "Version", "Header Length", "Differentiated Services Field", "DSCP", "ECN",
      "Total Length", "Identification", "Flags", "Reserved bit", "Don't fragment",
      "More fragments", "Fragment offset", "Time to live", "Protocol", "Header checksum",
      "Source Address", "Destination Address",
    ]);
    expect(decoded.layers[2].fields.find((item) => item.label === "Window")?.value).toBe("64240");
    expect(decoded.layers[2].fields.find((item) => item.label === "SYN")?.value).toBe("Set");
  });

  it("recognizes ARP and raw IP fallback", () => {
    const arp = new Uint8Array(42);
    arp[12] = 8;
    arp[13] = 6;
    arp[20] = 0;
    arp[21] = 1;
    arp.set([192, 168, 1, 4], 28);
    arp.set([192, 168, 1, 1], 38);
    expect(dissect(arp).protocol).toBe("ARP");
    expect(dissect(arp).info).toBe("Who has 192.168.1.1? Tell 192.168.1.4");
    expect(dissect(new Uint8Array([0x45]), 101).protocol).toBe("Data");
  });

  it("decodes a DNS question with Wireshark style Info text", () => {
    const dns = new Uint8Array([
      0x1a, 0x2b, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0,
      7, 101, 120, 97, 109, 112, 108, 101, 3, 99, 111, 109, 0, 0, 1, 0, 1,
    ]);
    const udp = new Uint8Array(8 + dns.length);
    const udpView = new DataView(udp.buffer);
    udpView.setUint16(0, 53000);
    udpView.setUint16(2, 53);
    udpView.setUint16(4, udp.length);
    udp.set(dns, 8);
    const decoded = dissect(ethernetIpv4(17, udp));
    expect(decoded.protocol).toBe("DNS");
    expect(decoded.info).toBe("Standard query 0x1a2b A example.com");
    expect(decoded.layers.at(-1)?.fields.some((item) => item.label === "Question 1")).toBe(true);
  });
});
