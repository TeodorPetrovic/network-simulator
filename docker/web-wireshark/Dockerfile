FROM debian:trixie

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl gnupg x11-utils xauth xvfb wireshark && \
    install -d -m 0755 /etc/apt/keyrings && \
    curl -fsSL https://xpra.org/gpg.asc | gpg --dearmor -o /etc/apt/keyrings/xpra.gpg && \
    echo "deb [signed-by=/etc/apt/keyrings/xpra.gpg] https://xpra.org/ trixie main" > /etc/apt/sources.list.d/xpra.list && \
    apt-get update && apt-get install -y --no-install-recommends xpra xpra-x11 && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 --shell /bin/bash gns3 && \
    install -d -o 1000 -g 1000 -m 0700 /run/user/1000 && \
    install -d -o 1000 -g 1000 -m 1777 /tmp/.X11-unix

ENV HOME=/home/gns3 XDG_RUNTIME_DIR=/run/user/1000
USER 1000:1000
CMD ["tail", "-f", "/dev/null"]
