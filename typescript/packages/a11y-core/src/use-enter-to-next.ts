import { useCallback, useRef } from 'react';
import { Platform, type TextInput, type TextInputProps } from 'react-native';

export interface RegisterFieldOptions {
  multiline?: boolean;
}

export type RegisteredFieldProps = Pick<TextInputProps, 'onSubmitEditing' | 'returnKeyType'> & {
  ref: (node: TextInput | null) => void;
};

export interface EnterToNextResult {
  registerField: (index: number, options?: RegisterFieldOptions) => RegisteredFieldProps;
  submit: () => void;
}

/**
 * Makes Enter walk a web form field by field and submit from the last one.
 * Multiline fields keep their newline behavior and are skipped on the way.
 */
export function useEnterToNext(fieldCount: number, onSubmit: () => void): EnterToNextResult {
  const fieldRefs = useRef<(TextInput | null)[]>([]);
  const multilineFields = useRef(new Set<number>());

  const submit = useCallback(() => {
    onSubmit();
  }, [onSubmit]);

  const registerField = useCallback(
    (index: number, options: RegisterFieldOptions = {}): RegisteredFieldProps => {
      if (options.multiline === true) multilineFields.current.add(index);
      else multilineFields.current.delete(index);

      const ref = (node: TextInput | null) => {
        fieldRefs.current[index] = node;
      };
      if (Platform.OS !== 'web' || options.multiline === true) return { ref };

      const lastSingleLineField = Array.from({ length: fieldCount }, (_, candidate) => candidate)
        .reverse()
        .find((candidate) => !multilineFields.current.has(candidate));

      return {
        ref,
        returnKeyType: index === lastSingleLineField ? 'done' : 'next',
        onSubmitEditing: () => {
          for (let nextIndex = index + 1; nextIndex < fieldCount; nextIndex += 1) {
            if (multilineFields.current.has(nextIndex)) continue;
            const nextField = fieldRefs.current[nextIndex];
            if (nextField) {
              nextField.focus();
              return;
            }
          }
          submit();
        },
      };
    },
    [fieldCount, submit],
  );

  return { registerField, submit };
}
