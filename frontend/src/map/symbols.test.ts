import { describe, expect, it } from "vitest";
import { resolveCanvasSymbol } from "./symbols";

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
});
