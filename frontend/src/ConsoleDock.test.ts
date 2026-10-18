import { describe, expect, it, vi } from "vitest";
import { dockerResizeControlFrame, sendDockerResizeControl } from "./ConsoleDock";

describe("Docker console resize control frame", () => {
  it("matches the text frame accepted by the Docker console WebSocket", () => {
    expect(dockerResizeControlFrame(24, 80)).toBe('{"type":"resize","rows":24,"columns":80}');
  });

  it("sends a resize only for positive dimensions on an open socket", () => {
    const send = vi.fn();
    const socket = { readyState: WebSocket.OPEN, send };

    expect(sendDockerResizeControl(socket, 40, 120)).toBe(true);
    expect(send).toHaveBeenCalledWith('{"type":"resize","rows":40,"columns":120}');
    expect(sendDockerResizeControl(socket, 0, 120)).toBe(false);
    expect(sendDockerResizeControl({ ...socket, readyState: WebSocket.CONNECTING }, 40, 120)).toBe(false);
    expect(send).toHaveBeenCalledTimes(1);
  });
});
