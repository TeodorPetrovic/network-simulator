import type { Gns3Appliance, RuntimeTemplate } from "./api";
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import "./App.test-utils";
import { api } from "./api";
import { renderRoutedApp } from "./App.test-utils";
import { renderWorkspace } from "./App.test-utils";
import { ApplianceManagerPage } from "./ApplianceManagerPage";

describe("appliances", () => {
    it("opens the dedicated appliance manager route from the projects page", async () => {
      vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
      const router = renderRoutedApp();
      fireEvent.click(await screen.findByRole("button", { name: "Appliance manager" }));
      expect(await screen.findByRole("heading", { name: "Appliance library" })).toBeInTheDocument();
      expect(router.state.location.pathname).toBe("/appliances");
    });

    it("creates a node from an installed Docker image template", async () => {
      const template = {
        template_id: "template-1",
        name: "alpine:latest",
        template_type: "docker" as const,
        image: "alpine:latest",
        image_digest: "sha256:abcd",
        adapters: 8,
        memory: 0,
        cpus: 0,
      };
      vi.mocked(api.templates).mockResolvedValue([template]);
      vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
        node_id: "node-1",
        name: "Router A",
        node_type: "docker",
        image: "alpine:latest",
        status: "stopped",
        x: -420,
        y: -210,
      });
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
      fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
        target: { value: "Router A" },
      });
      fireEvent.change(screen.getByRole("textbox", { name: "Container command (optional)" }), {
        target: { value: "/bin/sh -c 'sleep 3600'" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Create node" }));
      expect(await screen.findByRole("button", { name: "Node Router A" })).toBeInTheDocument();
      expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "template-1", {
        name: "Router A",
        x: -420,
        y: -210,
        start_command: "/bin/sh -c 'sleep 3600'",
      });
    });

    it("explains the real Docker image prerequisite when no image is installed", async () => {
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      expect(await screen.findByText(/No local templates/)).toBeInTheDocument();
      expect(screen.queryByText(/placeholder/i)).not.toBeInTheDocument();
    });

    it("shows local appliance image availability, suggests iShare2 matches, downloads one, and imports the definition", async () => {
      const listing = {
        file_name: "alpine-linux-virt.gns3a",
        name: "Alpine Linux Virt",
        registry_path: "appliances/alpine-linux-virt.gns3a",
        source_url: "https://github.com/GNS3/gns3-registry/blob/master/appliances/alpine-linux-virt.gns3a",
        download_url: "https://raw.githubusercontent.com/GNS3/gns3-registry/master/appliances/alpine-linux-virt.gns3a",
        imported: false,
      };
      const detail = {
        file_name: listing.file_name,
        appliance_id: "5899d1b7-672d-4e21-920d-908d254cb88c",
        name: "Alpine Linux Virt",
        category: "guest",
        description: "Small Alpine Linux virtual machine.",
        status: "stable",
        registry_version: 1,
        template_type: "qemu",
        imported: false,
        source_url: listing.source_url,
        required_files: [{ filename: "alpine-virt-3.19.1-x86_64.qcow2", version: "3.19.1", size: 10000000, md5sum: null, download_url: null }],
      };
      const suggestedImage = { id: 42, name: "Alpine virt 3.19.1", image_type: "qemu", size: "120 MB", total_size: 120000000, file_count: 1, files: ["alpine-virt-3.19.1-x86_64.qcow2"], catalog: "iShare2 / LabHub" };
      const localTemplate = { template_id: "alpine-qemu", name: "alpine-virt-3.19.1-x86_64.qcow2", template_type: "qemu" as const, hda_disk_image: "/global/images/qemu/alpine-virt-3.19.1-x86_64.qcow2", adapters: 8, memory: 512, cpus: 1 };
      vi.mocked(api.searchGns3Appliances).mockResolvedValue([listing]);
      vi.mocked(api.gns3Appliance).mockResolvedValue(detail);
      vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([localTemplate]);
      vi.mocked(api.searchIshareImages).mockResolvedValue([suggestedImage]);
      vi.mocked(api.downloadIshareImage).mockResolvedValue({ id: 42, name: suggestedImage.name, image_type: "qemu", files: ["alpine-virt-3.19.1-x86_64.qcow2"], source_files: ["alpine-virt-3.19.1-x86_64.qcow2"], image_directory: "/global/images/qemu" });
      vi.mocked(api.importGns3Appliance).mockResolvedValue({ appliance: { ...detail, imported: true }, matching_local_templates: 1, message: "Appliance imported and matched to installed runtime images" });
      renderWorkspace(null);
      fireEvent.click(await screen.findByRole("button", { name: "Image catalog" }));
      await screen.findByRole("textbox", { name: "Search catalog" }, { timeout: 15_000 });
      expect(screen.getByLabelText("GNS3 appliance results").closest('[data-slot="scroll-area"]')).toHaveClass("h-full");
      fireEvent.change(screen.getByRole("textbox", { name: "Search catalog" }), { target: { value: "alpine" } });
      fireEvent.click(screen.getByRole("button", { name: "Search" }));
      fireEvent.click(await screen.findByRole("button", { name: /Alpine Linux Virt/ }));
  
      expect(await screen.findByText("alpine-virt-3.19.1-x86_64.qcow2")).toBeInTheDocument();
      expect(screen.getByText("Missing locally")).toBeInTheDocument();
      expect(await screen.findByText("Alpine virt 3.19.1")).toBeInTheDocument();
      expect(screen.getByText("Includes required file: alpine-virt-3.19.1-x86_64.qcow2")).toBeInTheDocument();
      expect(api.searchIshareImages).toHaveBeenCalledWith("alpine-virt-3.19.1-x86_64.qcow2", "qemu");
      fireEvent.click(screen.getByRole("button", { name: "Download" }));
      expect(await screen.findByText("Available locally")).toBeInTheDocument();
      expect(api.downloadIshareImage).toHaveBeenCalledWith(suggestedImage);
      expect(screen.getByText(/required vendor images are supplied separately/i)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Import .gns3a definition" }));
      expect(await screen.findByText(/1 local runtime template matched/)).toBeInTheDocument();
      expect(api.importGns3Appliance).toHaveBeenCalledWith("alpine-linux-virt.gns3a");
    }, 20_000);

    it("downloads supported iShare2 images into the global image library", async () => {
      const image = { id: 42, name: "alpine-virt", image_type: "qemu", size: "120 MB", total_size: 120000000, file_count: 1, files: ["alpine-virt-disk.qcow2"], catalog: "iShare2 / LabHub" };
      vi.mocked(api.searchIshareImages).mockResolvedValue([image]);
      vi.mocked(api.downloadIshareImage).mockResolvedValue({ id: 42, name: "alpine-virt", image_type: "qemu", files: ["alpine-virt-disk.qcow2"], source_files: ["alpine-virt-disk.qcow2"], image_directory: "/global/images/qemu" });
      renderWorkspace(null);
      fireEvent.click(await screen.findByRole("button", { name: "Image catalog" }));
      await screen.findByRole("textbox", { name: "Search catalog" });
      fireEvent.mouseDown(screen.getByRole("tab", { name: "iShare2 images" }), { button: 0, ctrlKey: false });
      fireEvent.change(screen.getByRole("textbox", { name: "Search catalog" }), { target: { value: "alpine" } });
      fireEvent.click(screen.getByRole("button", { name: "Search" }));
  
      await waitFor(() => expect(api.searchIshareImages).toHaveBeenCalledWith("alpine", "all"));
      expect(await screen.findByText("alpine-virt")).toBeInTheDocument();
      expect(screen.getByText(/downloaded into this controller’s global image library/i)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Download" }));
      expect(await screen.findByText(/Download complete: alpine-virt/)).toBeInTheDocument();
      expect(api.downloadIshareImage).toHaveBeenCalledWith(image);
      expect(api.searchIshareImages).toHaveBeenCalledWith("alpine", "all");
    });

    it("lists imported appliance configs and requires an explicit override for a mismatched image", async () => {
      const appliance = {
        file_name: "test-router.gns3a",
        appliance_id: "38aa3135-f169-4131-9b64-73605787b5ef",
        name: "Test Router",
        category: "router",
        description: "Router appliance with a versioned image slot.",
        status: "stable",
        registry_version: 8,
        template_type: "qemu",
        imported: true,
        source_url: "https://github.com/GNS3/gns3-registry",
        required_files: [],
        versions: [{
          name: "1.0",
          settings: "default",
          template_type: "qemu",
          runtime_supported: true,
          images: [{
            key: "hda_disk_image",
            filename: "official-router.qcow2",
            assignable: true,
            filesize: 20,
            md5sum: "expected-md5",
            assigned_image_id: null,
            assigned_image_name: null,
            suggested_image_id: "qemu-custom",
            suggested_image_name: "custom-router.qcow2",
            image_path: null,
            actual_size: 20,
            actual_md5sum: "actual-md5",
            actual_sha1sum: "actual-sha1",
            status: "mismatch",
            allow_checksum_mismatch: false,
          }],
        }],
      };
      const localImage = { image_id: "qemu-custom", name: "custom-router.qcow2", template_type: "qemu" as const, path: "/global/qemu/custom-router.qcow2", size: 20, md5sum: "actual-md5", sha1sum: "actual-sha1" };
      vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [appliance], local_images: [localImage], custom_image_options: [] });
      vi.mocked(api.setApplianceImageBinding).mockResolvedValue({ appliance });
      render(<ApplianceManagerPage onBack={vi.fn()} />);
  
      expect((await screen.findAllByText("Test Router")).length).toBeGreaterThan(0);
      expect(await screen.findByText("Checksum or size mismatch")).toBeInTheDocument();
      expect(screen.getByText("expected-md5")).toBeInTheDocument();
      expect(screen.getByText("actual-md5")).toBeInTheDocument();
      const override = screen.getByRole("checkbox", { name: /allow this image despite checksum or size mismatch/i });
      fireEvent.click(override);
      fireEvent.click(screen.getByRole("button", { name: "Assign image" }));
  
      await waitFor(() => expect(api.setApplianceImageBinding).toHaveBeenCalledWith("test-router.gns3a", {
        version: "1.0",
        image_key: "hda_disk_image",
        image_id: "qemu-custom",
        allow_checksum_mismatch: true,
      }));
    });

    it("keeps discovered images out of the appliance library until a custom appliance is created", async () => {
      const alpineTemplate: RuntimeTemplate = {
        template_id: "alpine-docker-template",
        name: "alpine:latest",
        template_type: "docker",
        image: "alpine:latest",
        image_digest: "sha256:alpine-image-digest",
        start_command: "",
        environment: "",
        extra_hosts: "",
        extra_volumes: [],
        adapters: 1,
        memory: 0,
        cpus: 0,
      };
      const mikrotikTemplate: RuntimeTemplate = {
        template_id: "mikrotik-qemu-template",
        name: "MikroTik CHR",
        template_type: "qemu",
        hda_disk_image: "/global/images/qemu/mikrotik-chr-7.23.8.qcow2",
        adapters: 2,
        memory: 256,
        cpus: 1,
        adapter_type: "virtio-net-pci",
        hda_disk_interface: "virtio",
        boot_priority: "c",
      };
      const manager = {
        appliances: [],
        local_images: [{ image_id: mikrotikTemplate.template_id, name: "mikrotik-chr-7.23.8.qcow2", template_type: "qemu" as const, path: mikrotikTemplate.hda_disk_image, size: 45_000_000, md5sum: null, sha1sum: null }],
        custom_image_options: [alpineTemplate, mikrotikTemplate],
      };
      const customAppliance: Gns3Appliance = {
        file_name: "custom-mikrotik.gns3a",
        appliance_id: "custom-mikrotik-id",
        name: "MikroTik CHR lab",
        origin: "custom",
        category: "guest",
        description: "Custom appliance created in the appliance library.",
        status: null,
        registry_version: 8,
        template_type: "qemu",
        imported: true,
        source_url: "Custom appliance created in this library",
        required_files: [],
        versions: [{
          name: "Default",
          settings: "default",
          template_type: "qemu",
          runtime_supported: true,
          images: [{
            key: "hda_disk_image", filename: "mikrotik-chr-7.23.8.qcow2", assignable: true,
            filesize: 45_000_000, md5sum: null, assigned_image_id: mikrotikTemplate.template_id,
            assigned_image_name: "mikrotik-chr-7.23.8.qcow2", suggested_image_id: mikrotikTemplate.template_id,
            suggested_image_name: "mikrotik-chr-7.23.8.qcow2", image_path: mikrotikTemplate.hda_disk_image,
            actual_size: 45_000_000, actual_md5sum: null, actual_sha1sum: null,
            status: "size-verified", allow_checksum_mismatch: false,
          }],
        }],
      };
      const createdState = { ...manager, appliances: [customAppliance] };
      const applianceTemplate = { ...mikrotikTemplate, template_id: "custom-mikrotik-project-template", name: "MikroTik CHR lab · Default", appliance_id: customAppliance.appliance_id, appliance_version: "Default" } as RuntimeTemplate;
      vi.mocked(api.applianceManager).mockResolvedValueOnce(manager).mockResolvedValueOnce(createdState);
      vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([applianceTemplate]);
      vi.mocked(api.createCustomAppliance).mockResolvedValue(customAppliance);
      render(<ApplianceManagerPage onBack={vi.fn()} />);
  
      await screen.findByRole("heading", { name: "Appliances" });
      expect(screen.queryByRole("button", { name: /alpine:latest/ })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /mikrotik-chr-7\.23\.8/ })).not.toBeInTheDocument();
  
      fireEvent.click(screen.getByRole("button", { name: "Create custom appliance" }));
      await screen.findByRole("dialog", { name: "Create custom appliance" });
      fireEvent.change(screen.getByRole("textbox", { name: "Appliance name" }), { target: { value: "MikroTik CHR lab" } });
      fireEvent.click(screen.getByRole("button", { name: "Create appliance" }));
  
      await waitFor(() => expect(api.createCustomAppliance).toHaveBeenCalledWith({
        name: "MikroTik CHR lab",
        source_template_id: mikrotikTemplate.template_id,
        runtime_settings: {},
      }));
      expect(await screen.findByRole("button", { name: /MikroTik CHR lab/ })).toBeInTheDocument();
      expect(screen.getByText("Custom appliance")).toBeInTheDocument();
    });

    it("opens the global image library modal before any appliance has been imported", async () => {
      vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
      render(<ApplianceManagerPage onBack={vi.fn()} />);
  
      expect(await screen.findByRole("button", { name: /Global images/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Import .gns3a file" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Add QEMU image" })).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /Global images/ }));
      expect(await screen.findByRole("dialog", { name: "Global image library" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Add QEMU image" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Add Dynamips IOS image" })).toBeInTheDocument();
    });

    it("loads the GNS3 appliance catalog when its tab opens without requiring a search", async () => {
      const registryItem = {
        file_name: "alpine-linux-virt.gns3a",
        name: "Alpine Linux Virt",
        registry_path: "appliances/alpine-linux-virt.gns3a",
        source_url: "https://github.com/GNS3/gns3-registry/blob/master/appliances/alpine-linux-virt.gns3a",
        download_url: "https://raw.githubusercontent.com/GNS3/gns3-registry/master/appliances/alpine-linux-virt.gns3a",
        imported: false,
      };
      vi.mocked(api.applianceManager).mockResolvedValue({ appliances: [], local_images: [], custom_image_options: [] });
      vi.mocked(api.searchGns3Appliances).mockResolvedValue([registryItem]);
      render(<ApplianceManagerPage onBack={vi.fn()} />);
  
      fireEvent.mouseDown(await screen.findByRole("tab", { name: /GNS3 registry/ }), { button: 0, ctrlKey: false });
  
      await waitFor(() => expect(api.searchGns3Appliances).toHaveBeenCalledWith(""));
      expect(await screen.findByRole("button", { name: /Alpine Linux Virt/ })).toBeInTheDocument();
    });

    it("creates a QEMU node from an installed disk image without a container command", async () => {
      const template = {
        template_id: "qemu-template-1",
        name: "router.qcow2",
        template_type: "qemu" as const,
        hda_disk_image: "/images/qemu/router.qcow2",
        adapters: 8,
        memory: 512,
        cpus: 1,
      };
      vi.mocked(api.templates).mockResolvedValue([template]);
      vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
        node_id: "vm-1",
        name: "Router VM",
        node_type: "qemu",
        status: "stopped",
        hda_disk_image: "/images/qemu/router.qcow2",
        x: -420,
        y: -210,
      });
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
      expect(screen.getByText(/Creates a private copy-on-write disk/)).toBeInTheDocument();
      fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
        target: { value: "Router VM" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Create node" }));
      expect(await screen.findByRole("button", { name: "Node Router VM" })).toBeInTheDocument();
      expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "qemu-template-1", {
        name: "Router VM",
        x: -420,
        y: -210,
      });
    });

    it("uploads a custom QEMU image into the global library and selects its real template", async () => {
      const template = {
        template_id: "uploaded-qemu-template",
        name: "custom-router.qcow2",
        template_type: "qemu" as const,
        hda_disk_image: "/global/images/qemu/custom-router.qcow2",
        adapters: 8,
        memory: 512,
        cpus: 1,
      };
      vi.mocked(api.uploadQemuImage).mockResolvedValue({ name: "custom-router.qcow2", image: template.hda_disk_image, template_type: "qemu", bytes: 1024 });
      vi.mocked(api.templates).mockResolvedValueOnce([]).mockResolvedValueOnce([]).mockResolvedValueOnce([template]);
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      const image = new File([new Uint8Array([1, 2, 3])], "custom-router.qcow2");
      fireEvent.change(screen.getByLabelText("Add a QEMU disk image"), { target: { files: [image] } });
  
      expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
      expect(screen.getByRole("textbox", { name: "Node name" })).toHaveValue("custom-router");
      expect(await screen.findByText(/global image library/)).toBeInTheDocument();
      expect(api.uploadQemuImage).toHaveBeenCalledWith(image);
    });

});
