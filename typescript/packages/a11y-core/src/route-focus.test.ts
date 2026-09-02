// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  createRouteFocusController,
  type RouteFocusController,
  type RouteFocusRuntime,
  type RouteFocusTarget,
} from './route-focus.js';

const FRAME_MS = 16;
const RETRY_TIMEOUT_MS = 100;
const controllers: RouteFocusController[] = [];

function runtime(): RouteFocusRuntime {
  return {
    document,
    requestAnimationFrame: (callback) =>
      window.setTimeout(() => {
        callback(performance.now());
      }, FRAME_MS),
    cancelAnimationFrame: (handle) => {
      window.clearTimeout(handle);
    },
    setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
    clearTimeout: (handle) => {
      window.clearTimeout(handle);
    },
    now: () => Date.now(),
  };
}

function controller(): RouteFocusController {
  const instance = createRouteFocusController(runtime(), {
    focusRetryTimeoutMs: RETRY_TIMEOUT_MS,
  });
  controllers.push(instance);
  return instance;
}

function focusable<K extends keyof HTMLElementTagNameMap>(tag: K): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.tabIndex = -1;
  document.body.append(element);
  return element;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
});

afterEach(() => {
  for (const instance of controllers.splice(0)) instance.dispose();
  document.body.innerHTML = '';
  vi.useRealTimers();
});

describe('createRouteFocusController', () => {
  it('waits for the entry target to mount and restores the initiating control', () => {
    const instance = controller();
    const trigger = focusable('button');
    trigger.focus();
    let heading: HTMLHeadingElement | null = null;

    const leaveRoute = instance.enterRoute(() => heading);
    vi.advanceTimersByTime(FRAME_MS);

    expect(document.activeElement).toBe(trigger);

    heading = focusable('h1');
    vi.advanceTimersByTime(FRAME_MS);

    expect(document.activeElement).toBe(heading);

    leaveRoute();
    vi.advanceTimersByTime(FRAME_MS);

    expect(document.activeElement).toBe(trigger);
  });

  it('keeps the real return target after an inert transition blurs focus to body', () => {
    const instance = controller();
    const scene = document.createElement('section');
    const trigger = document.createElement('button');
    scene.append(trigger);
    document.body.append(scene);
    trigger.focus();
    const focusTrigger = vi.spyOn(trigger, 'focus');

    scene.setAttribute('inert', '');
    trigger.blur();
    expect(document.activeElement).toBe(document.body);

    const heading = focusable('h1');
    const leaveRoute = instance.enterRoute(() => heading);
    vi.advanceTimersByTime(FRAME_MS);
    leaveRoute();
    vi.advanceTimersByTime(FRAME_MS);

    expect(focusTrigger).not.toHaveBeenCalled();

    scene.removeAttribute('inert');
    vi.advanceTimersByTime(FRAME_MS);

    expect(focusTrigger).toHaveBeenCalledWith({ preventScroll: true });
    expect(document.activeElement).toBe(trigger);
  });

  it('stops retrying after the configured timeout', () => {
    const instance = controller();
    const getTarget = vi.fn<() => RouteFocusTarget | null>(() => null);

    instance.enterRoute(getTarget);
    vi.advanceTimersByTime(RETRY_TIMEOUT_MS + FRAME_MS);
    const callsAtTimeout = getTarget.mock.calls.length;
    vi.advanceTimersByTime(RETRY_TIMEOUT_MS * 2);

    expect(callsAtTimeout).toBeGreaterThan(0);
    expect(getTarget).toHaveBeenCalledTimes(callsAtTimeout);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('does not steal focus when the user moves to another reachable control', () => {
    const instance = controller();
    const trigger = focusable('button');
    trigger.focus();
    let heading: HTMLHeadingElement | null = null;
    instance.enterRoute(() => heading);
    vi.advanceTimersByTime(FRAME_MS);

    const input = focusable('input');
    input.focus();
    heading = focusable('h1');
    const focusHeading = vi.spyOn(heading, 'focus');
    vi.advanceTimersByTime(FRAME_MS);

    expect(focusHeading).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(input);
  });

  it('never focuses targets under inert or aria-hidden ancestors', () => {
    const instance = controller();
    const trigger = focusable('button');
    trigger.focus();
    const inertScene = document.createElement('section');
    inertScene.setAttribute('inert', '');
    const inertTarget = document.createElement('h1');
    inertTarget.tabIndex = -1;
    inertScene.append(inertTarget);
    document.body.append(inertScene);
    const hiddenScene = document.createElement('section');
    hiddenScene.setAttribute('aria-hidden', 'true');
    const hiddenTarget = document.createElement('h1');
    hiddenTarget.tabIndex = -1;
    hiddenScene.append(hiddenTarget);
    document.body.append(hiddenScene);
    let target: RouteFocusTarget = inertTarget;
    const focusInert = vi.spyOn(inertTarget, 'focus');
    const focusHidden = vi.spyOn(hiddenTarget, 'focus');

    instance.enterRoute(() => target);
    vi.advanceTimersByTime(FRAME_MS);
    target = hiddenTarget;
    vi.advanceTimersByTime(FRAME_MS);

    expect(focusInert).not.toHaveBeenCalled();
    expect(focusHidden).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(trigger);
  });
});
