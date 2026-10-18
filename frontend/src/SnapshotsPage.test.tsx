import { cleanup, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it as registerTest, vi } from "vitest";
import { api } from "./api";
import { SnapshotsPage } from "./SnapshotsPage";

const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("./PagesShell", () => ({
  PagesShell: ({ children }: { children: ReactNode }) => <main>{children}</main>,
  PageTitle: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("SnapshotsPage", () => {
  it("renders the backend epoch-seconds timestamp as the snapshot date", async () => {
    const createdAt = 1_797_000_000;
    vi.spyOn(api, "snapshots").mockResolvedValue([{
      snapshot_id: "snapshot-1",
      name: "Before changes",
      created_at: createdAt,
    }]);
    vi.spyOn(api, "topology").mockResolvedValue({
      project: { project_id: "p1", name: "Core lab" },
      nodes: [],
      links: [],
      drawings: [],
      event_cursor: 0,
    });

    render(<SnapshotsPage projectId="p1" />);

    expect(await screen.findByRole("button", { name: "Restore" })).toBeInTheDocument();
    expect(screen.getByText(new Date(createdAt * 1000).toLocaleString())).toBeInTheDocument();
    expect(screen.queryByText(new Date(createdAt).toLocaleString())).not.toBeInTheDocument();
  });
});
