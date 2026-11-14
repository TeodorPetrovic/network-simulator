import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/react";

configure({ asyncUtilTimeout: 10_000 });

Object.defineProperty(window, "scrollTo", {
  configurable: true,
  value: () => undefined,
});

Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
  configurable: true,
  value: () => null,
});

for (const method of ["hasPointerCapture", "setPointerCapture", "releasePointerCapture"] as const) {
  Object.defineProperty(Element.prototype, method, {
    configurable: true,
    value: () => method === "hasPointerCapture" ? false : undefined,
  });
}

Object.defineProperty(Element.prototype, "scrollIntoView", {
  configurable: true,
  value: () => undefined,
});

// Radix controls (Checkbox, Switch, RadioGroup) measure themselves with ResizeObserver, which jsdom lacks.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
}

// Sonner (toasts) and theme.ts read prefers-color-scheme through matchMedia, which jsdom lacks.
if (typeof window.matchMedia === "undefined") {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({ matches: false, media: query, onchange: null, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false }),
  });
}
