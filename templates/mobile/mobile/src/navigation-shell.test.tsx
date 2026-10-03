{% raw %}
import { fireEvent, render, screen } from "@testing-library/react-native";
import { usePathname, useRouter } from "expo-router";
import { NavigationBar, WorkspacePicker } from "./navigation-shell";
import { lightTheme } from "./theme";

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
beforeEach(() => {
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
    color: lightTheme.color.onAccent,
  });
  expect(screen.getByText("⌂", { includeHiddenElements: true })).toHaveStyle({
    color: lightTheme.color.text,
  });
  await fireEvent.press(screen.getByRole("link", { name: "Workspace" }), {
    nativeEvent: {},
    preventDefault: jest.fn(),
  });
  expect(navigate).toHaveBeenCalledWith("/workspace/privacy");
  const links = screen.getAllByRole("link");
  expect(links.at(-1)).toBe(screen.getByRole("link", { name: "Profile" }));
});
it("lets the section picker choose a page directly", async () => {
  await render(<WorkspacePicker />);
  await fireEvent.press(
    screen.getByRole("button", { name: "Workspace, Items" }),
  );
  await fireEvent.press(screen.getByRole("menuitem", { name: "Privacy" }), {
    nativeEvent: {},
    preventDefault: jest.fn(),
  });
  expect(navigate).toHaveBeenCalledWith("/workspace/privacy");
});
{% endraw %}
