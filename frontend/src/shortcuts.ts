export type ShortcutEntry = { keys: string[]; description: string; group: string };

// Only shortcuts that are implemented in code. Keep in sync with:
// map/useCanvasKeyboardShortcuts.ts, map/TopologyCanvas.tsx, SimulatorWorkspace.tsx,
// capture/CaptureViewer.tsx and copilot/CopilotWindow.tsx.
export const SHORTCUTS: ShortcutEntry[] = [
  { keys: ["?"], description: "Show this keyboard shortcuts list", group: "General" },
  { keys: ["Escape"], description: "Clear the selection, or cancel link drawing", group: "General" },
  { keys: ["Ctrl / ⌘ + click"], description: "Add a device to the selection", group: "Project map" },
  { keys: ["Drag empty canvas"], description: "Select the devices inside a rectangle", group: "Project map" },
  { keys: ["Space + drag", "Middle button drag"], description: "Pan the map", group: "Project map" },
  { keys: ["Ctrl / ⌘ + A"], description: "Select all devices", group: "Project map" },
  { keys: ["Ctrl / ⌘ + C"], description: "Copy the selected devices", group: "Project map" },
  { keys: ["Ctrl / ⌘ + V"], description: "Paste the copied devices", group: "Project map" },
  { keys: ["Ctrl / ⌘ + Z"], description: "Undo the last device move", group: "Project map" },
  { keys: ["Ctrl / ⌘ + +"], description: "Zoom in", group: "Project map" },
  { keys: ["Ctrl / ⌘ + -"], description: "Zoom out", group: "Project map" },
  { keys: ["↑", "↓", "←", "→"], description: "Nudge the selected unlocked devices by 10 px (not while the project is locked)", group: "Project map" },
  { keys: ["Delete", "Backspace"], description: "Delete the selected device, link or drawing", group: "Project map" },
  { keys: ["Enter", "Space"], description: "Select the focused device or link", group: "Project map" },
  { keys: ["Ctrl + F"], description: "Focus the packet filter", group: "Packet capture" },
  { keys: ["↑", "↓"], description: "Select the previous or next packet", group: "Packet capture" },
  { keys: ["Home", "End"], description: "Select the first or last packet", group: "Packet capture" },
  { keys: ["Alt + F"], description: "Follow the selected packet's stream", group: "Packet capture" },
  { keys: ["Enter"], description: "Send the message to the AI copilot", group: "AI copilot" },
  { keys: ["Shift + Enter"], description: "New line in the copilot message", group: "AI copilot" },
];
