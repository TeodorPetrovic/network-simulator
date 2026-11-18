#!/gns3/bin/busybox sh
# Container entrypoint shim modelled on GNS3's docker init.sh: wait until the
# simulator has created every Ethernet adapter (eth0..ethN-1), apply /etc/network/interfaces
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

# Wait for all eth available. As in GNS3, the controller creates eth0..eth{N-1} for every
# adapter (linked or not) right after the container starts; /gns3/expected-eth lists them and
# GNS3_MAX_ETHERNET names the last one. Bounded (2 min) so a failed attach cannot hang boot.
waited=0
while [ "$waited" -lt 240 ]
do
    missing=0
    for dev in `cat /gns3/expected-eth 2>/dev/null` $GNS3_MAX_ETHERNET
    do
        grep -q "^ *$dev:" /proc/net/dev || missing=1
    done
    [ "$missing" = 0 ] && break
    usleep 500000
    waited=$((waited + 1))
done

# activate eth interfaces
sed -n 's/^ *\(eth[0-9]*\):.*/\1/p' < /proc/net/dev | while read dev; do
	ip link set dev $dev up
done

# configure network interfaces
ifup -a -f

exec su ${GNS3_USER-root} -p -- /gns3/run-cmd.sh "$OLD_PATH" "$@"
