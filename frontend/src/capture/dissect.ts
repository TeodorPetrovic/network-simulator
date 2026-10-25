import { dissectArp } from "./dissectors/arp";
import { dissectCdp } from "./dissectors/cdp";
import { dissectDhcp } from "./dissectors/dhcp";
import { dissectDns } from "./dissectors/dns";
import { dissectEthernet } from "./dissectors/ethernet";
import { dissectIcmp } from "./dissectors/icmp";
import { dissectIcmpv6 } from "./dissectors/icmpv6";
import { dissectIpv4 } from "./dissectors/ipv4";
import { dissectIpv6 } from "./dissectors/ipv6";
import { dissectLldp } from "./dissectors/lldp";
import { dissectOspf } from "./dissectors/ospf";
import { dissectRip } from "./dissectors/rip";
import { dissectStp } from "./dissectors/stp";
import { dissectTcp } from "./dissectors/tcp";
import { dissectUdp } from "./dissectors/udp";
import { dissectVlan } from "./dissectors/vlan";
import { dissectVrrp } from "./dissectors/vrrp";
import { field, type Layer } from "./dissectors/types";

export type { Field, Layer } from "./dissectors/types";

export type Dissection = {
  layers: Layer[];
  protocol: string;
  src: string;
  dst: string;
  info: string;
};

function dataLayer(data: Uint8Array): Layer {
  return {
    name: "Data",
    summary: `${data.length} bytes`,
    fields: [field("Data", `${data.length} bytes`, 0, data.length)],
  };
}

export function dissect(data: Uint8Array, linkType = 1): Dissection {
  const layers: Layer[] = [];
  let offset = 0;
  let etherType = 0;
  let protocol = "Data";
  let source = "";
  let destination = "";
  let info = "";

  if (linkType === 1) {
    const ethernet = dissectEthernet(data);
    if (!ethernet) return { layers: [dataLayer(data)], protocol, src: source, dst: destination, info: `${data.length} bytes` };
    layers.push(ethernet.layer);
    offset = ethernet.nextOffset ?? 0;
    etherType = ethernet.nextProtocol ?? 0;

    if (etherType === 0x8100 || etherType === 0x88a8) {
      const vlan = dissectVlan(data, offset);
      if (vlan) {
        layers.push(vlan.layer);
        offset = vlan.nextOffset ?? offset;
        etherType = vlan.nextProtocol ?? etherType;
      }
    }
  } else if (linkType === 101) {
    etherType = (data[0] >> 4) === 4 ? 0x0800 : (data[0] >> 4) === 6 ? 0x86dd : 0;
  } else if (linkType === 113 || linkType === 276) {
    if (data.length >= 16) {
      etherType = (data[14] << 8) | data[15];
      offset = 16;
    }
  } else {
    return { layers: [dataLayer(data)], protocol, src: source, dst: destination, info: `${data.length} bytes` };
  }

  if (etherType === 0x0806) {
    const arp = dissectArp(data, offset);
    if (arp) {
      layers.push(arp.layer);
      protocol = arp.protocol;
      info = arp.info;
    }
  } else if (etherType === 0x0800) {
    const ip = dissectIpv4(data, offset);
    if (ip) {
      layers.push(ip.layer);
      source = ip.source ?? "";
      destination = ip.destination ?? "";
      offset = ip.nextOffset ?? offset;
      const ipProtocol = ip.nextProtocol ?? -1;
      protocol = "IPv4";

      if (ipProtocol === 6) {
        const tcp = dissectTcp(data, offset);
        if (tcp) {
          layers.push(tcp.layer);
          offset = tcp.nextOffset ?? offset;
          protocol = "TCP";
          info = tcp.info;
          const sourcePort = Number(tcp.source);
          const destinationPort = Number(tcp.destination);
          if (sourcePort === 53 || destinationPort === 53) {
            const dnsOffset = offset + 2;
            const dns = dissectDns(data, dnsOffset, "TCP");
            if (dns) {
              layers.push(dns.layer);
              protocol = dns.protocol;
              info = dns.info;
            }
          }
        }
      } else if (ipProtocol === 17) {
        const udp = dissectUdp(data, offset);
        if (udp) {
          layers.push(udp.layer);
          offset = udp.nextOffset ?? offset;
          protocol = "UDP";
          info = udp.info;
          const sourcePort = Number(udp.source);
          const destinationPort = Number(udp.destination);
          if (sourcePort === 53 || destinationPort === 53) {
            const dns = dissectDns(data, offset);
            if (dns) {
              layers.push(dns.layer);
              protocol = dns.protocol;
              info = dns.info;
            }
          } else if ([67, 68].includes(sourcePort) || [67, 68].includes(destinationPort)) {
            const dhcp = dissectDhcp(data, offset);
            if (dhcp) {
              layers.push(dhcp.layer);
              protocol = dhcp.protocol;
              info = dhcp.info;
            }
          } else if ([520].includes(sourcePort) || [520].includes(destinationPort)) {
            const rip = dissectRip(data, offset);
            if (rip) {
              layers.push(rip.layer);
              protocol = rip.protocol;
              info = rip.info;
            }
          }
        }
      } else if (ipProtocol === 1) {
        const ttl = Number(ip.layer.fields.find((item) => item.label === "Time to live")?.value);
        const icmp = dissectIcmp(data, offset, ttl);
        if (icmp) {
          layers.push(icmp.layer);
          protocol = icmp.protocol;
          info = icmp.info;
        }
      } else if (ipProtocol === 89) {
        const ospf = dissectOspf(data, offset);
        if (ospf) { layers.push(ospf.layer); protocol = ospf.protocol; info = ospf.info; }
      } else if (ipProtocol === 112) {
        const vrrp = dissectVrrp(data, offset);
        if (vrrp) { layers.push(vrrp.layer); protocol = vrrp.protocol; info = vrrp.info; }
      }
    }
  } else if (etherType === 0x86dd) {
    const ip = dissectIpv6(data, offset);
    if (ip) {
      layers.push(ip.layer);
      source = ip.source ?? "";
      destination = ip.destination ?? "";
      offset = ip.nextOffset ?? offset;
      const nextHeader = ip.nextProtocol ?? -1;
      protocol = "IPv6";
      if (nextHeader === 6) {
        const tcp = dissectTcp(data, offset);
        if (tcp) {
          layers.push(tcp.layer); protocol = tcp.protocol; info = tcp.info;
          const sourcePort = Number(tcp.source); const destinationPort = Number(tcp.destination);
          if (sourcePort === 53 || destinationPort === 53) {
            const dns = dissectDns(data, (tcp.nextOffset ?? offset) + 2, "TCP");
            if (dns) { layers.push(dns.layer); protocol = dns.protocol; info = dns.info; }
          }
        }
      } else if (nextHeader === 17) {
        const udp = dissectUdp(data, offset);
        if (udp) {
          layers.push(udp.layer); protocol = udp.protocol; info = udp.info;
          const sourcePort = Number(udp.source); const destinationPort = Number(udp.destination);
          if (sourcePort === 53 || destinationPort === 53) {
            const dns = dissectDns(data, udp.nextOffset ?? offset);
            if (dns) { layers.push(dns.layer); protocol = dns.protocol; info = dns.info; }
          }
        }
      } else if (nextHeader === 58) {
        const icmp = dissectIcmpv6(data, offset);
        if (icmp) { layers.push(icmp.layer); protocol = icmp.protocol; info = icmp.info; }
      }
    }
  } else if (etherType === 0x88cc) {
    const lldp = dissectLldp(data, offset);
    if (lldp) { layers.push(lldp.layer); protocol = lldp.protocol; info = lldp.info; }
  } else if (etherType === 0x010b) {
    const stp = dissectStp(data, offset);
    if (stp) { layers.push(stp.layer); protocol = stp.protocol; info = stp.info; }
  } else if (etherType === 0x2000) {
    const cdp = dissectCdp(data, offset);
    if (cdp) { layers.push(cdp.layer); protocol = cdp.protocol; info = cdp.info; }
  }

  if (!info) info = source && destination ? `${source} → ${destination}` : protocol;
  return { layers, protocol, src: source, dst: destination, info };
}
