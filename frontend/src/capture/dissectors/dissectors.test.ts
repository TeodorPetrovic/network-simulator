import { describe, expect, it } from "vitest";
import { dissectCdp } from "./cdp";
import { dissectDhcp } from "./dhcp";
import { dissectIcmp } from "./icmp";
import { dissectIcmpv6 } from "./icmpv6";
import { dissectIpv6 } from "./ipv6";
import { dissectLldp } from "./lldp";
import { dissectOspf } from "./ospf";
import { dissectRip } from "./rip";
import { dissectStp } from "./stp";
import { dissectVlan } from "./vlan";
import { dissectVrrp } from "./vrrp";

describe("protocol dissectors", () => {
  it("decodes IPv6 and ICMPv6 fixture headers", () => {
    const ipv6 = new Uint8Array(40);
    ipv6[0] = 0x60;
    ipv6.set([0x20, 1, 0x0d, 0xb8], 8);
    ipv6.set([0x20, 1, 0x0d, 0xb8], 24);
    ipv6[6] = 58;
    ipv6[7] = 32;
    expect(dissectIpv6(ipv6, 0)?.protocol).toBe("IPv6");
    expect(dissectIpv6(ipv6, 0)?.layer.fields).toHaveLength(8);
    expect(dissectIcmpv6(new Uint8Array([128, 0, 0, 0, 0, 1, 0, 2]), 0)?.info).toContain("Echo (ping) request");
  });

  it("decodes VLAN, ICMP, and DHCP fixtures", () => {
    expect(dissectVlan(new Uint8Array([0xa0, 0, 8, 0]), 0)?.layer.fields.find((item) => item.label === "VLAN ID")?.value).toBe("0");
    expect(dissectIcmp(new Uint8Array([8, 0, 0, 0, 0, 1, 0, 1]), 0, 64)?.info).toContain("ttl=64");
    const dhcp = new Uint8Array(244);
    dhcp[0] = 1;
    dhcp[236] = 99;
    dhcp[237] = 130;
    dhcp[238] = 83;
    dhcp[239] = 99;
    dhcp.set([53, 1, 1, 255], 240);
    expect(dissectDhcp(dhcp, 0)?.info).toBe("DHCP Discover");
  });

  it("decodes STP, LLDP, and CDP fixture fields", () => {
    expect(dissectStp(new Uint8Array([0, 0, 0, 0]), 0)?.protocol).toBe("STP");
    const lldp = new Uint8Array([2, 7, 4, 0, 1, 2, 3, 4, 5, 4, 2, 5, 120, 6, 2, 0, 120, 10, 4, 110, 111, 100, 101, 0, 0]);
    expect(dissectLldp(lldp, 0)?.layer.fields.map((item) => item.label)).toContain("System Name");
    expect(dissectCdp(new Uint8Array([2, 180, 0, 0, 0, 1, 0, 8, 65, 66, 67, 68]), 0)?.info).toBe("Device ID: ABCD");
  });

  it("decodes VRRP, OSPF, and RIP fixture headers", () => {
    const vrrp = new Uint8Array([0x21, 1, 100, 1, 0, 1, 0, 0, 10, 0, 0, 1]);
    expect(dissectVrrp(vrrp, 0)?.info).toContain("vrid 1");
    const ospf = new Uint8Array(24);
    ospf[0] = 2;
    ospf[1] = 1;
    ospf[3] = 24;
    expect(dissectOspf(ospf, 0)?.info).toBe("Hello Packet");
    const rip = new Uint8Array([1, 2, 0, 0]);
    expect(dissectRip(rip, 0)?.info).toBe("Request");
  });
});
