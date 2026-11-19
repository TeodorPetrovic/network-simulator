# Vežba 8 – provera u simulatoru na pravim uređajima

Topologija iz zadatka (R1 = MikroTik CHR 7.23.8, SW1 i SW2 = **SONiC-VS VM** (master, `sonic-vs.img.gz`, QEMU/KVM,
2 GB RAM), dva VPCS i NAT čvor) napravljena je u simulatoru, a svi blokovi iz `task/resenje.frag.html` izvršeni su
redom kroz konzole simulatora (naredba po naredbu). Pun izlaz: `izvrsenje-resenja.log`.

Rezultat: sve komande prolaze bez greške. VPC1 dobija 10.10.10.199/24 (gateway 10.10.10.1) od R1; VPC1↔VPC2 ping radi
(TTL 63, ruter na štapu); 8.8.8.8 radi kroz NAT; 10.10.99.1 odgovara, a 10.10.99.2 je blokiran firewall-om (timeout);
R1 vidi SW1 i SW2 u `/ip neighbor`, SW1 vidi R1 i SW2 u `show lldp table`; backup i export fajlovi su napravljeni.
Napomena: RouterOS 7.23 pri prvoj prijavi traži novu lozinku (u zadatku je to `Vezba8admin`).
