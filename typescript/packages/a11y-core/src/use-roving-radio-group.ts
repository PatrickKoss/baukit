import { useRef } from 'react';
import type { View } from 'react-native';

import { asFocusTarget } from './dom-boundary.js';

export interface RovingKeyEvent {
  nativeEvent: { key: string };
  preventDefault: () => void;
}

export interface RovingRadioGroupOptions<T> {
  onChange: (value: T) => void;
  options: readonly T[];
  value: T;
}

export interface RovingRadioProps {
  onKeyDown: (event: RovingKeyEvent) => void;
  ref: (node: View | null) => void;
  tabIndex: 0 | -1;
}

export interface RovingRadioGroupResult {
  radioProps: (index: number) => RovingRadioProps;
}

/** Maps an arrow/Home/End key to the option it should move to, or null to ignore it. */
export function nextIndexFor(key: string, index: number, length: number): number | null {
  switch (key) {
    case 'ArrowDown':
    case 'ArrowRight':
      return (index + 1) % length;
    case 'ArrowLeft':
    case 'ArrowUp':
      return (index - 1 + length) % length;
    case 'Home':
      return 0;
    case 'End':
      return length - 1;
    default:
      return null;
  }
}

/** Gives a radio group one tab stop and arrow-key movement between its options. */
export function useRovingRadioGroup<T>({
  onChange,
  options,
  value,
}: RovingRadioGroupOptions<T>): RovingRadioGroupResult {
  const optionRefs = useRef<(View | null)[]>([]);
  const selectedIndex = Math.max(
    0,
    options.findIndex((option) => Object.is(option, value)),
  );

  const onKeyDown = (event: RovingKeyEvent, index: number) => {
    if (options.length === 0) return;
    const nextIndex = nextIndexFor(event.nativeEvent.key, index, options.length);
    if (nextIndex === null) return;

    event.preventDefault();
    const nextValue = options[nextIndex];
    if (nextValue !== undefined) onChange(nextValue);
    asFocusTarget({ current: optionRefs.current[nextIndex] ?? null })?.focus();
  };

  return {
    radioProps: (index: number) => ({
      onKeyDown: (event: RovingKeyEvent) => {
        onKeyDown(event, index);
      },
      ref: (node: View | null) => {
        optionRefs.current[index] = node;
      },
      tabIndex: index === selectedIndex ? 0 : -1,
    }),
  };
}
