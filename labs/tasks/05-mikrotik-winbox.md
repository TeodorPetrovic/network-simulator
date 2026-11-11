# Lab 5: Managing a MikroTik router with WinBox

**File:** `05-mikrotik-winbox.gns3project` · **Time:** 45 min · **Nodes:** MikroTik CHR, WinBox client, PC1, switch, NAT

## Scenario
A RouterOS CHR (10.5.0.1/24 on ether1) is managed from a graphical WinBox client on the same LAN (10.5.0.10). ether2 goes to the NAT node and gives the router Internet access. The disk already ships with this configuration.

## Tasks
1. Start the nodes. Open the WinBox console (VNC) and connect to `10.5.0.1`, user `admin`, empty password.
2. Set an admin password and rename the router identity.
3. Review `IP > Addresses`, `IP > Routes` and `IP > Firewall > NAT`. Explain each entry.
4. Give PC1 an address (`ip 10.5.0.20/24 10.5.0.1`) and ping the router. Then ping an Internet address; this should fail until you add the right NAT and route. Find what is missing, if anything, and fix it in WinBox.
5. Create a DHCP server on ether1 (pool 10.5.0.100-150) and verify with `dhcp` on PC1.
6. Add a firewall rule that drops ICMP from PC1 to the router, test it, then remove it.
7. Export the configuration (`/export`) from the terminal.

## Deliverables
- Screenshots of the WinBox address list and the DHCP lease for PC1.
- The exported configuration with your changes.
- One paragraph: why does WinBox need TCP 8291?
