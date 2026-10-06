{% raw %}
import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { Platform } from "react-native";
import { usePathname, useRouter } from "expo-router";
import { NavigationBar, WorkspacePicker } from "./navigation-shell";
import { lightTheme } from "./theme";
import { initializeI18n } from "./localization/i18n";
import { blendColors } from "@baukit/ui-tokens";

jest.mock("expo-router", () => ({
  usePathname: jest.fn(),
  useRouter: jest.fn(),
}));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 12, left: 0, right: 0 }),
}));
jest.mock("./theme", () => {
  const theme = jest.requireActual<typeof import("./theme")>("./theme");
  return { ...theme, useTheme: () => ({ theme: theme.lightTheme }) };
});
const navigate = jest.fn();
beforeEach(async () => {
  await initializeI18n("en");
  jest.mocked(useRouter).mockReturnValue({
    ...jest.requireActual<typeof import("expo-router")>("expo-router").router,
    navigate,
  });
  jest.mocked(usePathname).mockReturnValue("/workspace/items");
  navigate.mockClear();
});
it("rotates the current compact section and keeps the profile last", async () => {
  await render(
    <NavigationBar
      profile={{ label: "Profile", initials: "A", href: "/profile" }}
    />,
  );
  expect(screen.getByText("☷", { includeHiddenElements: true })).toHaveStyle({
    color: lightTheme.color.accent,
  });
  expect(screen.getByText("⌂", { includeHiddenElements: true })).toHaveStyle({
    color: lightTheme.color.text,
  });
  expect(screen.getByRole("link", { name: "Workspace" })).toHaveStyle({
    backgroundColor: blendColors(
      lightTheme.color.accent, lightTheme.color.surface, 0.14,
    ),
  });
  await fireEvent.press(screen.getByRole("link", { name: "Workspace" }), {
    nativeEvent: {},
    preventDefault: jest.fn(),
  });
  expect(navigate).toHaveBeenCalledWith("/workspace/privacy");
  const links = screen.getAllByRole("link");
  expect(links.at(-1)).toBe(screen.getByRole("link", { name: "Profile" }));
});
it("lets the section picker choose a page after closing on Android", async () => {
  jest.replaceProperty(Platform, "OS", "android");
  await render(<WorkspacePicker />);
  await fireEvent.press(
    screen.getByRole("button", { name: "Workspace, Items" }),
  );
  await fireEvent.press(screen.getByRole("menuitem", { name: "Privacy" }), {
    nativeEvent: {},
    preventDefault: jest.fn(),
  });
  expect(navigate).not.toHaveBeenCalled();
  expect(screen.queryByRole("menu")).toBeNull();
  await act(async () => {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        resolve();
      }),
    );
  });
  expect(navigate).toHaveBeenCalledWith("/workspace/privacy");
  jest.restoreAllMocks();
});
it("renders German navigation and menu controls from the catalog", async () => {
  await initializeI18n("de");
  await render(
    <>
      <NavigationBar />
      <WorkspacePicker />
    </>,
  );
  expect(screen.getByTestId("primary-navigation")).toHaveProp(
    "accessibilityLabel",
    "Hauptnavigation",
  );
  expect(screen.getByRole("link", { name: "Heute" })).toBeOnTheScreen();
  await fireEvent.press(
    screen.getByRole("button", { name: "Arbeitsbereich, Einträge" }),
  );
  expect(screen.getByRole("menuitem", { name: "Datenschutz" })).toBeOnTheScreen();
  expect(screen.getByRole("button", { name: "Menü schließen" })).toBeOnTheScreen();
});
it("uses the semantic danger token for destructive profile actions", async () => {
  await render(
    <NavigationBar
      profile={{
        label: "Account",
        initials: "A",
        menu: [
          { id: "signout", label: "Sign out", tone: "danger", onSelect: jest.fn() },
        ],
      }}
    />,
  );
  await fireEvent.press(screen.getByRole("button", { name: "Account" }));
  expect(screen.getByText("Sign out")).toHaveStyle({ color: lightTheme.color.error });
});
{% endraw %}
