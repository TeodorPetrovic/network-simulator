import { describe, expect, it } from "vitest";
import { defaultCanvasSymbolAsset, resolveCanvasSymbol } from "./symbols";

const defaults = { router: "router.svg", switch: "switch.svg", cloud: "cloud.svg", pc: "pc.svg" };

describe("canvas node symbols", () => {
  it("uses custom symbols from the controller library when selected", () => {
    expect(resolveCanvasSymbol("qemu", "custom-router.svg", defaults, ["custom-router.svg"])).toEqual({
      kind: "custom",
      symbolId: "custom-router.svg",
    });
  });

  it("matches GNS3 default symbol IDs and chooses a default for nodes without one", () => {
    expect(resolveCanvasSymbol("docker", ":/symbols/router.svg", defaults, [])).toEqual({ kind: "default", name: "router" });
    expect(resolveCanvasSymbol("vpcs", undefined, defaults, [])).toEqual({ kind: "default", name: "pc" });
    expect(resolveCanvasSymbol("unknown", undefined, defaults, [])).toEqual({ kind: "type" });
  });

  it("uses GNS3 artwork for the built-in node categories", () => {
    expect(defaultCanvasSymbolAsset("qemu", "pc")).toBe("qemu_guest.svg");
    expect(defaultCanvasSymbolAsset("vpcs", "pc")).toBe("vpcs_guest.svg");
    expect(defaultCanvasSymbolAsset("ethernet_switch", "switch")).toBe("ethernet_switch.svg");
    expect(defaultCanvasSymbolAsset("ethernet_hub", "switch")).toBe("hub.svg");
    expect(defaultCanvasSymbolAsset("nat", "cloud")).toBe("nat.svg");
  });
});
