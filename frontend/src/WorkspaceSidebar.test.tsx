import { describe, expect, it } from "vitest";
import { templateCategory } from "./WorkspaceSidebar";
import type { RuntimeTemplate } from "./api";

const template = (template_type: RuntimeTemplate["template_type"], name: string, category?: string) => ({ template_id: "t", name, template_type, ...(category ? { category } : {}) }) as RuntimeTemplate;

describe("templateCategory", () => {
  it("maps GNS3 runtime template types to their default device groups", () => {
    expect(templateCategory(template("dynamips", "Cisco IOS"))).toBe("routers");
    expect(templateCategory(template("ethernet_switch", "Ethernet switch"))).toBe("switches");
    expect(templateCategory(template("ethernet_hub", "Ethernet hub"))).toBe("switches");
    expect(templateCategory(template("vpcs", "VPCS"))).toBe("end-devices");
    expect(templateCategory(template("qemu", "Firewall VM", "Security"))).toBe("security");
  });
});
