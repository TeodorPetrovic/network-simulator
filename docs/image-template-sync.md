# Image backed device templates

At controller startup and after image uploads, the controller scans the QEMU and
Dynamips image directories and creates a template for any supported image that
is not already referenced by a template. MikroTik CHR images named
`mikrotik-chr-<version>.qcow2` receive CHR defaults; other QEMU disks receive a
generic QEMU template. Cisco IOS filenames identify the matching Dynamips
platform when possible, with c7200 used as a fallback for unknown IOS names.

Generated templates are stored with the controller templates and are created
only once. This preserves subsequent user edits. Template responses include a
computed `image_missing` boolean when a referenced QEMU or Dynamips image file
is no longer present. Missing image references do not cause user templates to
be deleted.
