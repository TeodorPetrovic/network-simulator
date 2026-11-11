// Student solution commands for labs 11 and 12 (typed from scratch on empty nodes).
export const lab11 = {
  sw1: [
    "hostname SW1", "no ip domain-lookup",
    "vlan 10", "name USERS", "vlan 20", "name SERVERS", "vlan 30", "name GUESTS", "exit",
    "interface FastEthernet1/0", "switchport mode trunk", "no shutdown", "exit",
    "interface FastEthernet1/1", "switchport mode access", "switchport access vlan 10", "no shutdown", "exit",
    "interface FastEthernet1/2", "switchport mode access", "switchport access vlan 20", "no shutdown", "exit",
    "interface FastEthernet1/3", "switchport mode access", "switchport access vlan 30", "no shutdown", "exit",
  ],
  r1: [
    "hostname R1", "no ip domain-lookup",
    "interface FastEthernet0/0", "no ip address", "no shutdown", "exit",
    "interface FastEthernet0/0.10", "encapsulation dot1Q 10", "ip address 10.11.10.1 255.255.255.0", "exit",
    "interface FastEthernet0/0.20", "encapsulation dot1Q 20", "ip address 10.11.20.1 255.255.255.0", "exit",
    "interface FastEthernet0/0.30", "encapsulation dot1Q 30", "ip address 10.11.30.1 255.255.255.0", "exit",
    "interface FastEthernet0/1", "ip address 10.11.100.1 255.255.255.252", "no shutdown", "exit",
    "ip route 0.0.0.0 0.0.0.0 10.11.100.2",
  ],
  isp: [
    "hostname ISP", "no ip domain-lookup",
    "interface FastEthernet0/0", "ip address 10.11.100.2 255.255.255.252", "no shutdown", "exit",
    "interface FastEthernet0/1", "ip address 172.16.99.1 255.255.255.0", "no shutdown", "exit",
    "ip route 10.11.0.0 255.255.0.0 10.11.100.1",
  ],
  pcs: { pc1: "ip 10.11.10.11/24 10.11.10.1", pc2: "ip 10.11.20.11/24 10.11.20.1", pc3: "ip 10.11.30.11/24 10.11.30.1", remote: "ip 172.16.99.10/24 172.16.99.1" },
};
export const lab12 = {
  r1: [
    "hostname R1-HQ", "no ip domain-lookup",
    "interface Loopback0", "ip address 1.1.1.1 255.255.255.255", "exit",
    "interface Loopback1", "ip address 203.0.113.1 255.255.255.255", "exit",
    "interface FastEthernet0/0", "ip address 10.12.1.1 255.255.255.0", "no shutdown", "exit",
    "interface FastEthernet0/1", "ip address 10.12.12.1 255.255.255.252", "no shutdown", "exit",
    "router ospf 1", "router-id 1.1.1.1",
    "network 10.12.1.0 0.0.0.255 area 0", "network 10.12.12.0 0.0.0.3 area 0", "network 1.1.1.1 0.0.0.0 area 0",
    "passive-interface FastEthernet0/0", "default-information originate always", "exit",
    "ip route 0.0.0.0 0.0.0.0 Loopback1",
  ],
  chr: [
    "/system identity set name=CHR-Branch",
    "/ip address add address=10.12.12.2/30 interface=ether1",
    "/ip address add address=10.12.2.1/24 interface=ether2",
    "/interface bridge add name=lo0",
    "/ip address add address=2.2.2.2/32 interface=lo0",
    "/routing ospf instance add name=o1 version=2 router-id=2.2.2.2",
    "/routing ospf area add name=backbone area-id=0.0.0.0 instance=o1",
    "/routing ospf interface-template add area=backbone networks=10.12.12.0/30",
    "/routing ospf interface-template add area=backbone networks=10.12.2.0/24 passive",
    "/routing ospf interface-template add area=backbone networks=2.2.2.2/32 passive",
  ],
  pcs: { pchq: "ip 10.12.1.11/24 10.12.1.1", pcbr: "ip 10.12.2.10/24 10.12.2.1" },
};
