import { render, screen } from "@testing-library/react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import SignInScreen from "../app/(auth)/sign-in";

jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: jest.fn(),
}));
jest.mock("./auth", () => ({
  useOidcAuth: () => ({ ready: true, signIn: jest.fn() }),
}));
jest.mock("./theme", () => {
  const theme = jest.requireActual<typeof import("./theme")>("./theme");
  return {
    ...theme,
    useTheme: () => ({ mode: "light", theme: theme.lightTheme }),
  };
});

it.each([0, 36])(
  "places the sign-in header below the %s dp top inset",
  async (top) => {
    jest
      .mocked(useSafeAreaInsets)
      .mockReturnValue({ top, bottom: 24, left: 0, right: 0 });
    await render(<SignInScreen />);
    expect(screen.getByTestId("sign-in-screen")).toHaveStyle({
      paddingTop: top,
    });
  },
);
