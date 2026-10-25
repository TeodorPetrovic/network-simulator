import { expect, it } from "vitest";
import { dissect } from "./dissect";
import { readPcap } from "./pcap";

function tcpPacket(source: [number, number, number, number], destination: [number, number, number, number], sourcePort: number, destinationPort: number, flags: number, payload = new Uint8Array(), options = new Uint8Array()): Uint8Array {
  const tcpHeaderLength = 20 + options.length;
  const ipv4Length = 20 + tcpHeaderLength + payload.length;
  const packet = new Uint8Array(14 + ipv4Length);
  packet.set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 0]);
  packet.set([0x45, 0, ipv4Length >> 8, ipv4Length & 0xff, 0, 1, 0, 0, 64, 6, 0, 0, ...source, ...destination], 14);
  const tcp = 34;
  const data = new DataView(packet.buffer);
  data.setUint16(tcp, sourcePort);
  data.setUint16(tcp + 2, destinationPort);
  packet[tcp + 12] = (tcpHeaderLength / 4) << 4;
  packet[tcp + 13] = flags;
  data.setUint16(tcp + 14, 64240);
  packet.set(options, tcp + 20);
  packet.set(payload, tcp + tcpHeaderLength);
  return packet;
}

function udpPacket(sourcePort: number, destinationPort: number, payload: Uint8Array): Uint8Array {
  const udp = new Uint8Array(8 + payload.length);
  const data = new DataView(udp.buffer);
  data.setUint16(0, sourcePort);
  data.setUint16(2, destinationPort);
  data.setUint16(4, udp.length);
  udp.set(payload, 8);
  return ipv4Packet(17, udp);
}

function ipv4Packet(protocol: number, payload: Uint8Array): Uint8Array {
  const packet = new Uint8Array(14 + 20 + payload.length);
  packet.set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 0]);
  packet.set([0x45, 0, 0, (20 + payload.length) & 0xff, 0, 1, 0, 0, 64, protocol, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2], 14);
  packet.set(payload, 34);
  return packet;
}

function arpPacket(): Uint8Array {
  const packet = new Uint8Array(42);
  packet.set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 6]);
  packet.set([0, 1, 8, 0, 6, 4, 0, 1, 6, 7, 8, 9, 10, 11, 10, 0, 0, 1, 0, 0, 0, 0, 0, 0, 10, 0, 0, 2], 14);
  return packet;
}

function pcap(packets: Uint8Array[]): Uint8Array {
  const size = 24 + packets.reduce((total, packet) => total + 16 + packet.length, 0);
  const bytes = new Uint8Array(size);
  bytes.set([0xd4, 0xc3, 0xb2, 0xa1]);
  const data = new DataView(bytes.buffer);
  data.setUint16(4, 2, true);
  data.setUint16(6, 4, true);
  data.setUint32(16, 65535, true);
  data.setUint32(20, 1, true);
  let offset = 24;
  packets.forEach((packet, index) => {
    data.setUint32(offset, 1 + index, true);
    data.setUint32(offset + 8, packet.length, true);
    data.setUint32(offset + 12, packet.length, true);
    bytes.set(packet, offset + 16);
    offset += 16 + packet.length;
  });
  return bytes;
}

it("smokes a generated pcap with TCP handshake and HTTP, DNS, ICMP, and ARP", async () => {
  const dns = new Uint8Array([
    0x1a, 0x2b, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    7, 101, 120, 97, 109, 112, 108, 101, 3, 99, 111, 109, 0, 0, 1, 0, 1,
  ]);
  const icmp = new Uint8Array([8, 0, 0, 0, 0, 1, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8]);
  const packets = [
    tcpPacket([10, 0, 0, 1], [10, 0, 0, 2], 49152, 80, 0x02, new Uint8Array(), new Uint8Array([2, 4, 5, 180])),
    tcpPacket([10, 0, 0, 2], [10, 0, 0, 1], 80, 49152, 0x12),
    tcpPacket([10, 0, 0, 1], [10, 0, 0, 2], 49152, 80, 0x10),
    tcpPacket([10, 0, 0, 1], [10, 0, 0, 2], 49152, 80, 0x18, new TextEncoder().encode("GET /")),
    udpPacket(53000, 53, dns),
    ipv4Packet(1, icmp),
    arpPacket(),
  ];
  const source = pcap(packets);
  const stream = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(source); controller.close(); } });
  const actual: string[] = [];
  for await (const packet of readPcap(stream)) actual.push(dissect(packet.data, packet.linkType).info);

  expect(actual).toEqual([
    "49152 → 80 [SYN] Seq=0 Win=64240 Len=0 MSS=1460",
    "80 → 49152 [SYN, ACK] Seq=0 Ack=0 Win=64240 Len=0",
    "49152 → 80 [ACK] Seq=0 Ack=0 Win=64240 Len=0",
    "49152 → 80 [ACK, PSH] Seq=0 Ack=0 Win=64240 Len=5",
    "Standard query 0x1a2b A example.com",
    "Echo (ping) request  id=0x0001, seq=1/8, ttl=64",
    "Who has 10.0.0.2? Tell 10.0.0.1",
  ]);
});
