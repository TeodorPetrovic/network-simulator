import { useRef } from "react";
import type { KeyboardEvent, MutableRefObject } from "react";
import type { Node } from "../api";

type Props = {
  nodes: Node[];
  selectedNodeIds: string[];
  projectLocked: boolean;
  onMultiSelect: (ids: string[]) => void;
  onZoom: (delta: number) => void;
  onNodeMoveEnd: (positions: Array<{ id: string; x: number; y: number }>) => void;
  onDeleteSelected: () => void;
  onClearSelection: () => void;
  onCopy: () => void;
  onPaste: () => void;
};

type Point = { x: number; y: number };

export function useCanvasKeyboardShortcuts(props: Props, lastMoveRef: MutableRefObject<Map<string, Point> | null>) {
  const spaceDownRef = useRef(false);
  const onKeyDown = (event: KeyboardEvent<SVGSVGElement>) => {
    const mod = event.metaKey || event.ctrlKey;
    if (event.code === "Space") { event.preventDefault(); spaceDownRef.current = true; }
    if (mod && event.key.toLowerCase() === "c") { event.preventDefault(); props.onCopy(); }
    if (mod && event.key.toLowerCase() === "v") { event.preventDefault(); props.onPaste(); }
    if (mod && event.key.toLowerCase() === "a") { event.preventDefault(); props.onMultiSelect(props.nodes.map((node) => node.node_id)); }
    if (mod && (event.key === "+" || event.key === "=")) { event.preventDefault(); props.onZoom(0.1); }
    if (mod && event.key === "-") { event.preventDefault(); props.onZoom(-0.1); }
    if (mod && event.key.toLowerCase() === "z" && lastMoveRef.current) {
      event.preventDefault();
      props.onNodeMoveEnd(Array.from(lastMoveRef.current, ([id, point]) => ({ id, ...point })));
      lastMoveRef.current = null;
    }
    if (event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); props.onDeleteSelected(); }
    const movement: Record<string, Point> = { ArrowLeft: { x: -10, y: 0 }, ArrowRight: { x: 10, y: 0 }, ArrowUp: { x: 0, y: -10 }, ArrowDown: { x: 0, y: 10 } };
    const delta = movement[event.key];
    if (delta && props.selectedNodeIds.length > 0 && !mod && !props.projectLocked) {
      event.preventDefault();
      const positions = [];
      for (const id of props.selectedNodeIds) {
        const node = props.nodes.find((item) => item.node_id === id);
        if (node && !node.locked) positions.push({ id, x: Number(node.x ?? 0) + delta.x, y: Number(node.y ?? 0) + delta.y });
      }
      props.onNodeMoveEnd(positions);
    }
    if (event.key === "Escape") props.onClearSelection();
  };
  const clearSpace = (event?: KeyboardEvent<SVGSVGElement>) => {
    if (!event || event.code === "Space") spaceDownRef.current = false;
  };
  return { spaceDownRef, onKeyDown, onKeyUp: clearSpace, onBlur: () => clearSpace() };
}
