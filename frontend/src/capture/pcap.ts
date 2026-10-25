export type PcapPacket = { timestamp: number; data: Uint8Array; originalLength: number; linkType: number };
export type PcapHeader = { littleEndian: boolean; nanoseconds: boolean; linkType: number; snapLength: number };
export class PcapError extends Error {}

/** Parse a classic pcap ReadableStream incrementally. Packets are yielded as soon as complete. */
export async function* readPcap(stream: ReadableStream<Uint8Array>): AsyncGenerator<PcapPacket, PcapHeader> {
  const reader = stream.getReader(); let buffer = new Uint8Array(0); let header: PcapHeader | undefined;
  const concat = (a: Uint8Array, b: Uint8Array) => { const out = new Uint8Array(a.length + b.length); out.set(a); out.set(b, a.length); return out; };
  const u32 = (d: DataView, o: number, le: boolean) => d.getUint32(o, le);
  try {
    while (true) {
      const next = await reader.read(); if (!next.done) buffer = concat(buffer, next.value);
      if (!header && buffer.length >= 4) {
        const magic = [...buffer.slice(0, 4)].map((n) => n.toString(16).padStart(2,"0")).join("");
        const formats: Record<string, [boolean, boolean]> = { "d4c3b2a1":[true,false], "a1b2c3d4":[false,false], "4d3cb2a1":[true,true], "a1b23c4d":[false,true] };
        if (magic === "0a0d0d0a") throw new PcapError("pcapng files are not supported; open a classic .pcap capture.");
        const fmt = formats[magic]; if (!fmt) throw new PcapError("Unrecognized capture format (expected classic pcap).");
      }
      if (!header && buffer.length >= 24) {
        const magic = [...buffer.slice(0,4)].map(n=>n.toString(16).padStart(2,"0")).join(""); const [littleEndian,nanoseconds] = ({"d4c3b2a1":[true,false],"a1b2c3d4":[false,false],"4d3cb2a1":[true,true],"a1b23c4d":[false,true]} as Record<string,[boolean,boolean]>)[magic];
        const d = new DataView(buffer.buffer,buffer.byteOffset,buffer.byteLength); const linkType=u32(d,20,littleEndian);
        if (![1,101].includes(linkType)) throw new PcapError(`Unsupported pcap link type ${linkType}.`);
        header={littleEndian,nanoseconds,linkType,snapLength:u32(d,16,littleEndian)}; buffer=buffer.slice(24);
      }
      while (header && buffer.length >= 16) {
        const d=new DataView(buffer.buffer,buffer.byteOffset,buffer.byteLength); const seconds=u32(d,0,header.littleEndian), fraction=u32(d,4,header.littleEndian), included=u32(d,8,header.littleEndian), originalLength=u32(d,12,header.littleEndian);
        if (included > 64*1024*1024) throw new PcapError("Capture packet exceeds the 64 MiB safety limit.");
        if (buffer.length < 16+included) break;
        yield {timestamp:seconds+(header.nanoseconds?fraction/1e9:fraction/1e6),data:buffer.slice(16,16+included),originalLength,linkType:header.linkType}; buffer=buffer.slice(16+included);
      }
      if (next.done) break;
    }
    if (!header) throw new PcapError("Capture ended before the pcap header was complete.");
    if (buffer.length) throw new PcapError("Capture ended in the middle of a packet.");
    return header;
  } finally { reader.releaseLock(); }
}
