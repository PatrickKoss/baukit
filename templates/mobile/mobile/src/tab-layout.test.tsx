import { render } from "@testing-library/react-native";
import { Tabs } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import TabLayout from "../app/(tabs)/_layout";

jest.mock("expo-router", () => ({
  Tabs: Object.assign(
    jest.fn(() => null),
    { Screen: () => null },
  ),
}));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: jest.fn(),
}));
jest.mock("./theme", () => ({
  ...jest.requireActual<typeof import("./theme")>("./theme"),
  useTheme: () => ({
    theme: jest.requireActual<typeof import("./theme")>("./theme").lightTheme,
  }),
}));

it.each([0, 36])("places tab scenes below the %s dp top inset", async (top) => {
  jest
    .mocked(useSafeAreaInsets)
    .mockReturnValue({ top, bottom: 24, left: 0, right: 0 });
  await render(<TabLayout />);
  expect(jest.mocked(Tabs).mock.calls[0]?.[0]).toMatchObject({
    screenOptions: { sceneStyle: { paddingTop: top } },
  });
});
