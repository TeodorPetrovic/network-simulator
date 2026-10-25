# GNS3 3.1 parity audit notes

## VPCS console types

GNS3 3.1 VPCS uses the generic compute node console wrapper. Its wrapper supports
`telnet` and `ssh`; `none` disables the console. The wrapper accepts a Telnet
stream from the VPCS process and exposes it using the configured type. See
`compute/vpcs/vpcs_vm.py` and `compute/base_node.py` in the reference server at
`~/.cache/gns3-server-3.1/gns3server`.

This controller accepts all three values for VPCS nodes and templates. Its
browser WebSocket console negotiates the VPCS Telnet stream and bridges terminal
data directly; SSH is a supported configuration value, while the browser route
does not create a separate SSH listener. Other values are rejected with an
error listing the supported choices.
