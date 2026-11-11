# Lab 10: DHCP, DNS and NAT on a Linux gateway

**File:** `10-linux-services.gns3project` · **Time:** 60 min · **Nodes:** Gateway, Intranet web server, Client1, Client2, switch, NAT
**Requires:** the Docker image `ns-labs/services` (build: `docker build labs/docker/services -t ns-labs/services`).

## Scenario
The gateway (10.10.0.1) runs dnsmasq for DHCP (10.10.0.100–150) and DNS, and masquerades the LAN through its second interface to the NAT node. `intranet.lab` resolves to the internal web server at 10.10.0.80.

## Tasks
1. Start the nodes. On Client1 run `ip addr`, `ip route` and `cat /etc/resolv.conf`. Which values came from DHCP?
2. Read the gateway's dnsmasq configuration and the DHCP log. Find the lease handed to each client.
3. From a client resolve and fetch the intranet page: `nslookup intranet.lab` and `wget -qO- http://intranet.lab`.
4. Reach the Internet from a client (`wget -qO- http://example.com`). On the gateway show the NAT rule (`iptables -t nat -L -n -v`) and its counters.
5. Reserve a fixed address for Client2 by MAC address, restart dnsmasq, renew the lease and verify.
6. Add a DNS record `files.lab` pointing to the web server and test it.
7. Stop dnsmasq and describe what clients with an existing lease and new clients experience.

## Deliverables
- The client network settings and the lease entries.
- The edited dnsmasq config.
- A capture of the DHCP exchange (DISCOVER, OFFER, REQUEST, ACK).
