// React Native has no BroadcastChannel. Node's is ref'd once it has a listener, and
// @clerk/clerk-js opens one at import, so an in-band Jest run would never exit.
Reflect.deleteProperty(globalThis, 'BroadcastChannel');
