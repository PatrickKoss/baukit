// @vitest-environment jsdom
{% raw %}
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NavigationShell } from "./navigation-shell";

afterEach(cleanup);
it("rotates the active workspace tab and tracks browser history", () => {
  window.history.replaceState(null, "", "/");
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: 320,
  });
  render(
    <>
      <h1>App</h1>
      <h2 id="items-title">Items</h2>
      <h2 id="privacy-title">Privacy</h2>
      <NavigationShell />
    </>,
  );
  fireEvent.click(screen.getByRole("link", { name: "Workspace" }));
  expect(window.location.hash).toBe("#items-title");
  expect(
    screen
      .getByRole("link", { name: "Workspace" })
      .getAttribute("aria-current"),
  ).toBe("page");
  fireEvent.click(screen.getByRole("link", { name: "Workspace" }));
  expect(window.location.hash).toBe("#privacy-title");
  act(() => {
    window.history.replaceState(null, "", "/");
    window.dispatchEvent(new PopStateEvent("popstate"));
  });
  expect(
    screen.getByRole("link", { name: "Home" }).getAttribute("aria-current"),
  ).toBe("page");
});
it("wires direct section selection and product-provided profile actions", () => {
  window.history.replaceState(null, "", "/");
  const signOut = vi.fn();
  render(
    <NavigationShell
      profile={{
        label: "Account",
        initials: "A",
        menu: [{ id: "signout", label: "Sign out", onSelect: signOut }],
      }}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Workspace, Items" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Privacy" }));
  expect(window.location.hash).toBe("#privacy-title");
  fireEvent.click(screen.getByRole("button", { name: "Account" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Sign out" }));
  expect(signOut).toHaveBeenCalledOnce();
});
{% endraw %}
