# Web Wireshark backend

The controller starts a `gns3/web-wireshark:latest` container on demand for each project. It uses the `gns3-wireshark` Docker bridge (default subnet `172.31.0.0/22`) and starts one xpra display per captured link. Display and WebSocket port values come from a process-local allocator over 10000–19999. Stopping the last session schedules container removal after 60 seconds; closing or deleting the project removes it immediately.

The capture process reads the controller's PCAP stream using a short lived, path-bound, one-use access ticket. The ticket expires after 60 seconds and the user's JWT is never placed in the container. The controller proxies binary WebSocket frames between the browser and xpra.

If pulling the image fails, the controller builds `docker/web-wireshark/Dockerfile` (Debian trixie, Wireshark, xpra, Xvfb, curl, and X11 utilities). The image uses xpra.org's apt repository and does not use the Aliyun mirror.

This differs from GNS3 3.1 by using the existing one-use access ticket system instead of passing the user's JWT to the container. The controller exposes a `WebWireshark` settings section with the defaults `enabled`, `image`, `network_subnet`, `memory`, `cpus`, and `pids_limit`. The implementation currently uses these documented defaults when creating containers.

## Frontend browser client

The frontend vendors the browser client from [Xpra HTML5 v20](https://github.com/Xpra-org/xpra-html5/releases/tag/v20) under `frontend/public/xpra/`. The upstream repository's `LICENSE` identifies the client as Mozilla Public License 2.0; the verbatim notice is included at `frontend/public/xpra/LICENSE`. The vendored subset is under 3 MB and excludes source maps, audio decoders, and auxiliary pages not needed for the Wireshark desktop. Keep the version pinned when updating these assets and preserve the upstream license file.
