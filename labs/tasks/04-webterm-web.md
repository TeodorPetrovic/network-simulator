# Lab 4: Browsing internal and external web servers

**File:** `04-webterm-web.gns3project` · **Time:** 30 min · **Nodes:** Webterm, 2 web servers, switch, NAT

## Scenario
A student workstation (Webterm, a Linux desktop with Firefox) is on the 10.4.0.0/24 LAN with two internal web servers (10.4.0.80 and 10.4.0.81). Its second interface reaches the Internet through the NAT node.

## Tasks
1. Start all nodes and open the Webterm console (VNC).
2. Browse to `http://10.4.0.80` and `http://10.4.0.81`. Note what each server says.
3. Browse to `https://example.com`. Which interface carries this traffic? Check with `ip route` in a terminal.
4. Capture on the link between Webterm and the LAN switch while loading an internal page. Identify the ARP exchange, the TCP handshake and the HTTP request.
5. Add a third web server (Alpine appliance) at 10.4.0.82 and serve a page of your own.
6. Break it on purpose: give the new server a wrong subnet mask and explain why the browser fails.

## Deliverables
- Screenshots of both internal pages and the external page.
- The capture file with the three protocol exchanges marked.
- The routing table of the Webterm and a sentence on why two default routes are not used.
