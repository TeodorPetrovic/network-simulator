import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "../api";
import { WebWiresharkWindow } from "./WebWiresharkWindow";

const itLong = (name: string, test: () => unknown) => it(name, test, 15_000);

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("WebWiresharkWindow", () => {
  itLong("loads the vendored client with a socket path and retry controls", () => {
    render(<WebWiresharkWindow projectId="p1" linkId="l1" wsPath="/v3/projects/p1/links/l1/capture/web-wireshark" title="Web Wireshark" onClose={() => undefined} />);
    const frame = screen.getByTitle("Web Wireshark") as HTMLIFrameElement;
    expect(frame.src).toContain("/xpra/index.html?");
    expect(frame.src).toContain("encoding=auto");
    expect(frame.src).toContain("reconnect=true");
    expect(frame.src).toContain("clipboard=false");
    expect(screen.getByRole("status")).toHaveTextContent("Connecting to Web Wireshark");
    fireEvent.load(frame);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  itLong("restarts the server session and reloads the iframe", async () => {
    vi.spyOn(api, "restartWebWireshark").mockResolvedValue({ link_id: "l1", nodes: [] });
    render(<WebWiresharkWindow projectId="p1" linkId="l1" wsPath="/v3/projects/p1/links/l1/capture/web-wireshark" title="Web Wireshark" onClose={() => undefined} />);
    const before = screen.getByTitle("Web Wireshark");
    fireEvent.click(screen.getByRole("button", { name: "Restart Wireshark" }));
    await waitFor(() => expect(api.restartWebWireshark).toHaveBeenCalledWith("p1", "l1"));
    await waitFor(() => expect(screen.getByTitle("Web Wireshark")).not.toBe(before));
  });
});
