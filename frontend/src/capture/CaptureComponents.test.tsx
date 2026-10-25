import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { dissect } from "./dissect";
import type { PacketRecord } from "./filter";
import { CaptureFilterBar, CaptureStatusBar, CaptureToolbar, FollowStreamDialog, PacketDetails, PacketList, StatisticsPanel } from "./CaptureComponents";

function packet(number: number, data = new Uint8Array([
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 8, 0,
  0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2,
  0xc0, 0, 0, 80, 0, 0, 0, 0, 0, 0, 0, 0, 0x50, 2, 0xfa, 0xf0, 0, 0, 0, 0,
])): PacketRecord {
  return { number, time: number, timestamp: number, data, originalLength: data.length, linkType: 1, decoded: dissect(data) };
}

describe("capture viewer components", () => {
  it("virtualizes packet rows and applies a column filter from the context menu", () => {
    const onApplyFilter = vi.fn();
    render(<PacketList packets={Array.from({ length: 1000 }, (_, index) => packet(index + 1))} selectedNumber={1} relativeTime autoScroll={false} onSelect={() => {}} onApplyFilter={onApplyFilter} />);
    expect(screen.getAllByRole("row")).toHaveLength(85);
    fireEvent.contextMenu(screen.getAllByText("10.0.0.1")[0]);
    expect(onApplyFilter).toHaveBeenCalledWith("ip.addr == 10.0.0.1");
  });

  it("filters packet fields, highlights packet bytes, and applies field filters", () => {
    const onApplyFilter = vi.fn();
    const current = packet(1);
    render(<PacketDetails packet={current} onApplyFilter={onApplyFilter} />);
    fireEvent.contextMenu(screen.getByText("Source Address: 10.0.0.1"));
    expect(onApplyFilter).toHaveBeenCalledWith("ip.src == 10.0.0.1");
    expect(screen.getByLabelText("Byte 26: 0a")).toBeTruthy();
  });

  it("connects filter input, statistics, and follow stream controls", () => {
    const update = vi.fn();
    const current = packet(1);
    const { rerender } = render(<CaptureFilterBar value="" error="" onChange={update} />);
    fireEvent.change(screen.getByLabelText("Display filter"), { target: { value: "tcp" } });
    expect(update).toHaveBeenCalledWith("tcp");
    rerender(<StatisticsPanel packets={[current]} />);
    expect(screen.getByText("Protocol Hierarchy")).toBeTruthy();
    expect(screen.getByText("Conversations")).toBeTruthy();
    const close = vi.fn();
    rerender(<FollowStreamDialog packets={[current]} onClose={close} />);
    expect(screen.getByRole("dialog", { name: "Follow stream" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(close).toHaveBeenCalledOnce();
  });

  it("renders toolbar and status actions", () => {
    const onSave = vi.fn();
    render(<>
      <CaptureToolbar
        title="Capture"
        paused={false}
        autoScroll
        relativeTime
        hasDownload={false}
        fileInput={{ current: null }}
        onToggleTime={() => {}}
        onToggleScroll={() => {}}
        onTogglePause={() => {}}
        onFollow={() => {}}
        onClear={() => {}}
        onSave={onSave}
        onOpenSaved={() => {}}
      />
      <CaptureStatusBar status="Capturing" displayed={4} total={5} />
    </>);
    fireEvent.click(screen.getByRole("button", { name: "Save .pcap" }));
    expect(onSave).toHaveBeenCalledOnce();
    expect(screen.getByText("4 displayed / 5 packets")).toBeTruthy();
  });
});
