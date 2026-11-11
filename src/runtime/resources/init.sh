#!/gns3/bin/busybox sh
# Container entrypoint shim modelled on GNS3's docker init.sh: wait until the
# simulator has attached every Ethernet adapter, apply /etc/network/interfaces
# and then run the image's original entrypoint/command.
OLD_PATH="$PATH"
PATH=/gns3/bin:/tmp/gns3/bin:/sbin:$PATH

if [ ! -d /tmp/gns3/bin ]; then
	busybox mkdir -p /tmp/gns3/bin
	for applet in `busybox --list`
	do
	  ln -s /gns3/bin/busybox "/tmp/gns3/bin/$applet"
	done
fi

[ -s /etc/hosts ] || cat > /etc/hosts << __EOF__
127.0.1.1	$HOSTNAME
127.0.0.1	localhost
::1	localhost ip6-localhost ip6-loopback
fe00::0	ip6-localnet
ff00::0	ip6-mcastprefix
ff02::1	ip6-allnodes
ff02::2	ip6-allrouters
__EOF__
grep -q "[[:space:]]$HOSTNAME\$" /etc/hosts || echo "127.0.1.1	$HOSTNAME" >> /etc/hosts

ip link set dev lo up

# The controller lists the adapters that have links in /gns3/expected-eth (one per line) before
# starting the container. Wait for those, but never forever: unlinked adapters never appear.
waited=0
while [ "$waited" -lt 240 ]
do
    missing=0
    for dev in `cat /gns3/expected-eth 2>/dev/null`
    do
        grep -q "^ *$dev:" /proc/net/dev || missing=1
    done
    [ "$missing" = 0 ] && break
    usleep 500000
    waited=$((waited + 1))
done

sed -n 's/^ *\(eth[0-9]*\):.*/\1/p' < /proc/net/dev | while read dev; do
	ip link set dev $dev up
	touch "/tmp/gns3/seen_$dev"
done

ifup -a -f

# Adapters are attached when the peer node of a link is running, so some can appear
# after this point (GNS3 pre-creates them all). Bring late ones up and configure them.
(
	while true
	do
		sed -n 's/^ *\(eth[0-9]*\):.*/\1/p' < /proc/net/dev | while read dev; do
			[ -e "/tmp/gns3/seen_$dev" ] && continue
			touch "/tmp/gns3/seen_$dev"
			ip link set dev $dev up
			ifup -f $dev > /dev/null 2>&1
		done
		usleep 500000
	done
) &

exec su ${GNS3_USER-root} -p -- /gns3/run-cmd.sh "$OLD_PATH" "$@"
