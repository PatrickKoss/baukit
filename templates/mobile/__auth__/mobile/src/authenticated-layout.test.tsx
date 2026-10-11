import type { PropsWithChildren } from 'react';
import type { SessionExpiredReason } from '@baukit/auth-native';
import { act, render, waitFor } from '@testing-library/react-native';
import type { ProductAuth } from './auth';

let mockAuth: ProductAuth;
const mockProvider = jest.fn(({ children }: PropsWithChildren<{
  readonly profileId: string | undefined;
  readonly sessionExpiredReason: SessionExpiredReason | undefined;
}>) => children);
const mockAppShell = jest.fn(({ children }: PropsWithChildren<{
  readonly preferenceSubjectId?: string | null;
}>) => children);
jest.mock('./auth', () => ({
  AuthProvider: ({ children }: PropsWithChildren) => children,
  useAuth: () => mockAuth,
}));
jest.mock('./api', () => ({ currentUser: jest.fn() }));
jest.mock('./local-data', () => ({
  AuthenticatedLocalDataProvider: (props: Parameters<typeof mockProvider>[0]) => mockProvider(props),
}));
jest.mock('./app-shell', () => ({
  AppShell: (props: Parameters<typeof mockAppShell>[0]) => mockAppShell(props),
}));
jest.mock('./theme', () => {
  const theme = jest.requireActual<typeof import('./theme')>('./theme');
  return { ...theme, useTheme: () => ({ theme: theme.lightTheme }) };
});
jest.mock('expo-router', () => ({
  Stack: Object.assign(({ children }: PropsWithChildren) => children, {
    Protected: ({ children }: PropsWithChildren) => children,
    Screen: () => null,
  }),
}));

import RootLayout from '../app/_layout';
import { currentUser, type CurrentUser } from './api';

beforeEach(() => {
  mockAuth = {
    accessToken: 'token', subject: 'same-subject', ready: true, sessionExpired: false,
    signIn: jest.fn(() => Promise.resolve(undefined)),
    signOut: jest.fn(() => Promise.resolve(undefined)),
  };
});

it('waits for the server profile before selecting local data and preferences', async () => {
  let confirm: ((profile: CurrentUser) => void) | undefined;
  jest.mocked(currentUser).mockReturnValueOnce(new Promise((resolve) => { confirm = resolve; }));
  await render(<RootLayout />);
  expect(mockProvider).toHaveBeenLastCalledWith(expect.objectContaining({ profileId: undefined }));
  expect(mockAppShell).toHaveBeenLastCalledWith(expect.objectContaining({ preferenceSubjectId: null }));

  await act(() => { confirm?.({ id: 'server-profile', subject: 'same-subject' }); });

  expect(mockProvider).toHaveBeenLastCalledWith(expect.objectContaining({ profileId: 'server-profile' }));
  expect(mockAppShell).toHaveBeenLastCalledWith(expect.objectContaining({ preferenceSubjectId: 'server-profile' }));
});

it('selects a new profile after re-onboarding with the same auth subject', async () => {
  jest.mocked(currentUser).mockResolvedValueOnce({ id: 'old-profile', subject: 'same-subject' });
  const rendered = await render(<RootLayout />);
  await waitFor(() => {
    expect(mockProvider).toHaveBeenLastCalledWith(expect.objectContaining({ profileId: 'old-profile' }));
  });
  await rendered.unmount();
  mockAuth = { ...mockAuth, accessToken: 'fresh-token' };
  jest.mocked(currentUser).mockResolvedValueOnce({ id: 'new-profile', subject: 'same-subject' });
  await render(<RootLayout />);
  await waitFor(() => {
    expect(mockProvider).toHaveBeenLastCalledWith(expect.objectContaining({ profileId: 'new-profile' }));
  });
});

it('passes profile_erased through the auth layout after the tokens are cleared', async () => {
  jest.mocked(currentUser).mockResolvedValueOnce({ id: 'profile', subject: 'same-subject' });
  const rendered = await render(<RootLayout />);
  mockAuth = {
    ready: true, sessionExpired: true, sessionExpiredReason: 'profile_erased',
    signIn: mockAuth.signIn, signOut: mockAuth.signOut,
  };
  await rendered.rerender(<RootLayout />);
  expect(mockProvider).toHaveBeenLastCalledWith(expect.objectContaining({
    profileId: undefined, sessionExpiredReason: 'profile_erased',
  }));
});
