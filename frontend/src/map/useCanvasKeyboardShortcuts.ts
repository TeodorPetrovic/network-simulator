import { useRef } from "react";
import type { KeyboardEvent, MutableRefObject } from "react";
import type { Node } from "../api";

type Props = {
  nodes: Node[];
  onMultiSelect: (ids: string[]) => void;
  onZoom: (delta: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
  onDeleteSelected: () => void;
  onClearSelection: () => void;
};

type Point = { x: number; y: number };

export function useCanvasKeyboardShortcuts(props: Props, lastMoveRef: MutableRefObject<Map<string, Point> | null>) {
  const spaceDownRef = useRef(false);
  const onKeyDown = (event: KeyboardEvent<SVGSVGElement>) => {
    const mod = event.metaKey || event.ctrlKey;
    if (event.code === "Space") { event.preventDefault(); spaceDownRef.current = true; }
    if (mod && event.key.toLowerCase() === "a") { event.preventDefault(); props.onMultiSelect(props.nodes.map((node) => node.node_id)); }
    if (mod && (event.key === "+" || event.key === "=")) { event.preventDefault(); props.onZoom(0.1); }
    if (mod && event.key === "-") { event.preventDefault(); props.onZoom(-0.1); }
    if (mod && event.key.toLowerCase() === "z" && lastMoveRef.current) {
      event.preventDefault();
      lastMoveRef.current.forEach((point, id) => props.onNodeMoveEnd(id, point.x, point.y));
      lastMoveRef.current = null;
    }
    if (event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); props.onDeleteSelected(); }
    if (event.key === "Escape") props.onClearSelection();
  };
  const clearSpace = (event?: KeyboardEvent<SVGSVGElement>) => {
    if (!event || event.code === "Space") spaceDownRef.current = false;
  };
  return { spaceDownRef, onKeyDown, onKeyUp: clearSpace, onBlur: () => clearSpace() };
}
