import { describe, expect, it } from "vitest";
import { PcapError, readPcap } from "./pcap";
function fixture(little=true, nano=false) { const b=new Uint8Array(24+16+3); const d=new DataView(b.buffer); b.set(little ? nano?[0x4d,0x3c,0xb2,0xa1]:[0xd4,0xc3,0xb2,0xa1] : nano?[0xa1,0xb2,0x3c,0x4d]:[0xa1,0xb2,0xc3,0xd4]); d.setUint16(4,2,little); d.setUint16(6,4,little); d.setUint32(16,65535,little); d.setUint32(20,1,little); d.setUint32(24,7,little); d.setUint32(28,nano?500:500000,little); d.setUint32(32,3,little); d.setUint32(36,3,little); b.set([1,2,3],40); return b; }
function stream(chunks: Uint8Array[]) { return new ReadableStream<Uint8Array>({start(c){for(const x of chunks)c.enqueue(x);c.close();}}); }
describe("classic pcap reader",()=>{
 it("handles partial reads and both endian variants",async()=>{ for(const little of [true,false]) { const b=fixture(little); const iter=readPcap(stream([b.slice(0,9),b.slice(9,31),b.slice(31)])); const p=await iter.next(); expect(p.value).toMatchObject({timestamp:7.5,originalLength:3,linkType:1,data:new Uint8Array([1,2,3])}); expect((await iter.next()).done).toBe(true); } });
 it("supports nanosecond timestamps",async()=>{ const it=readPcap(stream([fixture(true,true)])); expect((await it.next()).value?.timestamp).toBe(7.0000005); });
 it("explains pcapng gracefully",async()=>{ const b=new Uint8Array([10,13,13,10,0,0,0,0]); await expect(readPcap(stream([b])).next()).rejects.toThrow(PcapError); });
});
