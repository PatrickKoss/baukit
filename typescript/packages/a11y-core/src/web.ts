/**
 * The entry point for a plain React web app. Nothing reachable from here
 * imports `react-native` at runtime, so a product without React Native in its
 * dependency tree can bundle it. React Native products import the package root
 * instead and get these exports plus the native ones.
 */

export * from './dom-boundary.js';
export * from './use-focus-trap.js';
export * from './use-inert.js';
export * from './use-single-flight.js';
