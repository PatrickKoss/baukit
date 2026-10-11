import { act, renderHook, waitFor } from '@testing-library/react-native';
import { PersistenceIdentityMismatchError } from '@baukit/data-contracts';

jest.mock('./api', () => ({ currentUser: jest.fn() }));

import { currentUser, type CurrentUser } from './api';
import { useServerProfile } from './server-profile';

const readProfile = jest.mocked(currentUser);

it('waits for /me and returns a new profile ID for the same auth subject', async () => {
  let confirm: ((profile: CurrentUser) => void) | undefined;
  readProfile.mockReturnValueOnce(new Promise((resolve) => { confirm = resolve; }));
  const { result, rerender } = await renderHook(
    ({ token, subject }: { readonly token: string | undefined; readonly subject: string | undefined }) =>
      useServerProfile(token, subject),
    { initialProps: { token: 'first-token', subject: 'same-subject' } },
  );
  expect(result.current.profile).toBeUndefined();
  await act(() => { confirm?.({ id: 'old-profile', subject: 'same-subject' }); });
  expect(result.current.profile?.id).toBe('old-profile');

  await rerender({ token: undefined, subject: undefined });
  expect(result.current.profile).toBeUndefined();
  readProfile.mockResolvedValueOnce({ id: 'new-profile', subject: 'same-subject' });
  await rerender({ token: 'fresh-token', subject: 'same-subject' });
  await waitFor(() => { expect(result.current.profile?.id).toBe('new-profile'); });
});

it('blocks a server response for a different auth subject', async () => {
  readProfile.mockResolvedValueOnce({ id: 'other-profile', subject: 'other-subject' });
  const { result } = await renderHook(() => useServerProfile('token', 'expected-subject'));
  await waitFor(() => { expect(result.current.error).toBeInstanceOf(PersistenceIdentityMismatchError); });
  expect(result.current.profile).toBeUndefined();
});

it.each([new Error('server unavailable'), 'unavailable'])('reports a failed server confirmation: %s', async (cause) => {
  readProfile.mockRejectedValueOnce(cause);
  const { result } = await renderHook(() => useServerProfile('token', 'subject'));
  await waitFor(() => { expect(result.current.error).toBeInstanceOf(Error); });
  expect(result.current.profile).toBeUndefined();
});

it('ignores an old confirmation after switching accounts', async () => {
  let confirm: ((profile: CurrentUser) => void) | undefined;
  readProfile.mockReturnValueOnce(new Promise((resolve) => { confirm = resolve; }));
  const { result, rerender } = await renderHook(
    ({ token, subject }: { readonly token: string; readonly subject: string }) => useServerProfile(token, subject),
    { initialProps: { token: 'old-token', subject: 'old-subject' } },
  );
  readProfile.mockResolvedValueOnce({ id: 'new-profile', subject: 'new-subject' });
  await rerender({ token: 'new-token', subject: 'new-subject' });
  await act(() => { confirm?.({ id: 'old-profile', subject: 'old-subject' }); });
  expect(result.current.profile?.id).toBe('new-profile');
});

it('ignores a failed confirmation after unmount', async () => {
  let reject: ((cause: Error) => void) | undefined;
  readProfile.mockReturnValueOnce(new Promise((_resolve, rejectPromise) => { reject = rejectPromise; }));
  const { result, unmount } = await renderHook(() => useServerProfile('token', 'subject'));
  await unmount();
  await act(() => { reject?.(new Error('late failure')); });
  expect(result.current.error).toBeUndefined();
});
