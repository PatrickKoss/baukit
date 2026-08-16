// @vitest-environment jsdom

import { cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { syncAriaHiddenInert, useInert } from "./use-inert";

afterEach(cleanup);

describe("inactive scene inertness", () => {
  it("makes an aria-hidden scene inert and restores it when revealed", () => {
    const scene = document.createElement("section");
    scene.setAttribute("aria-hidden", "true");
    document.body.append(scene);

    syncAriaHiddenInert(document.body);
    expect(scene.hasAttribute("inert")).toBe(true);

    scene.setAttribute("aria-hidden", "false");
    syncAriaHiddenInert(document.body);
    expect(scene.hasAttribute("inert")).toBe(false);
    scene.remove();
  });

  it("preserves inertness owned by another behavior", () => {
    const scene = document.createElement("section");
    scene.setAttribute("aria-hidden", "true");
    scene.setAttribute("inert", "");
    document.body.append(scene);

    syncAriaHiddenInert(document.body);
    scene.setAttribute("aria-hidden", "false");
    syncAriaHiddenInert(document.body);

    expect(scene.hasAttribute("inert")).toBe(true);
    scene.remove();
  });

  it("keeps an aria-hidden scene inert until an overlapping overlay lease ends", () => {
    const scene = document.createElement("section");
    scene.setAttribute("aria-hidden", "true");
    document.body.append(scene);
    syncAriaHiddenInert(document.body);

    const overlayElement = document.createElement("div");
    document.body.append(overlayElement);
    const overlay = renderHook(() => {
      useInert({ current: overlayElement }, true);
    });
    scene.setAttribute("aria-hidden", "false");
    syncAriaHiddenInert(document.body);

    expect(scene.hasAttribute("inert")).toBe(true);
    overlay.unmount();
    expect(scene.hasAttribute("inert")).toBe(false);
    overlayElement.remove();
    scene.remove();
  });
});
