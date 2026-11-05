import { describe, expect, it } from "vitest";
import { parseSseFrames } from "../api/copilot";
import { toDisplayItems } from "./CopilotWindow";

describe("copilot", () => {
  it("parses complete SSE frames and keeps the partial tail", () => {
    const { events, rest } = parseSseFrames('event: session\ndata: {"session_id":"s1"}\n\nevent: text\ndata: {"te');
    expect(events).toEqual([{ kind: "session", session_id: "s1" }]);
    expect(rest).toBe('event: text\ndata: {"te');
  });
  it("pairs tool calls with their results", () => {
    const items = toDisplayItems([
      { role: "user", content: [{ type: "text", text: "list" }] },
      { role: "assistant", content: [{ type: "tool_use", id: "c1", name: "project_list", input: {} }] },
      { role: "user", content: [{ type: "tool_result", tool_use_id: "c1", content: "[]" }] },
      { role: "assistant", content: [{ type: "text", text: "none" }] },
    ]);
    expect(items.map((i) => i.kind)).toEqual(["user", "tool", "assistant"]);
    expect(items[1]).toMatchObject({ name: "project_list", result: "[]" });
  });
});
