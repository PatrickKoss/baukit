it('matches the React Native runtime, which has no BroadcastChannel', () => {
  expect(typeof BroadcastChannel).toBe('undefined');
});
