{% raw %}
import {
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react-native";
import ProfileScreen from "../app/(tabs)/profile";
import { useOidcAuth } from "./auth";
import { defaultAppPreferences } from "./app-preferences";

jest.mock("./auth", () => ({ useOidcAuth: jest.fn() }));
jest.mock("./app-shell", () => ({ useAppPreferences: jest.fn() }));
jest.mock("./theme", () => {
  const theme = jest.requireActual<typeof import("./theme")>("./theme");
  return { ...theme, useTheme: () => ({ theme: theme.lightTheme }) };
});

import { useAppPreferences } from "./app-shell";

const resetPreferenceIdentity = jest.fn(() => Promise.resolve());
const signOut = jest.fn(() =>
  Promise.resolve({ providerLogout: "completed" } as const),
);
beforeEach(() => {
  jest.clearAllMocks();
  jest.mocked(useOidcAuth).mockReturnValue({
    subject: "subject-123",
    ready: true,
    sessionExpired: false,
    signIn: jest.fn(() => Promise.resolve(undefined)),
    signOut,
  });
  jest.mocked(useAppPreferences).mockReturnValue({
    analytics: undefined,
    consent: defaultAppPreferences.analyticsConsent,
    preferences: defaultAppPreferences,
    setConsent: jest.fn(() => Promise.resolve()),
    updatePreferences: jest.fn(() => Promise.resolve(defaultAppPreferences)),
    resetPreferenceIdentity,
  });
});

it("shows the profile identity and resets preferences before signing out", async () => {
  await render(<ProfileScreen />);
  expect(screen.getByText("Signed in as subject-123.")).toBeOnTheScreen();
  await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
  await waitFor(() => {
    expect(signOut).toHaveBeenCalledTimes(1);
  });
  expect(resetPreferenceIdentity).toHaveBeenCalledTimes(1);
  expect(resetPreferenceIdentity.mock.invocationCallOrder[0]).toBeLessThan(
    signOut.mock.invocationCallOrder[0] ?? 0,
  );
});

it("shows a preference reset failure and keeps the user signed in", async () => {
  resetPreferenceIdentity.mockRejectedValueOnce(
    new Error("Could not reset preferences."),
  );
  await render(<ProfileScreen />);
  await fireEvent.press(screen.getByRole("button", { name: "Sign out" }));
  expect(
    await screen.findByText("Could not reset preferences."),
  ).toBeOnTheScreen();
  expect(signOut).not.toHaveBeenCalled();
});
{% endraw %}
