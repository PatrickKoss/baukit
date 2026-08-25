export interface BackOrReplaceNavigation {
  readonly canGoBack: () => boolean;
  readonly back: () => void;
  readonly replace: (destination: string) => void;
}

/** Uses real history when present, otherwise terminates at a semantic destination. */
export function backOrReplace(
  navigation: BackOrReplaceNavigation,
  fallbackDestination: string,
): void {
  if (navigation.canGoBack()) {
    navigation.back();
    return;
  }
  navigation.replace(fallbackDestination);
}

export function browserNavigation(browser: Window = window): BackOrReplaceNavigation {
  return {
    canGoBack: () => browser.history.length > 1,
    back: () => {
      browser.history.back();
    },
    replace: (destination) => {
      browser.location.replace(destination);
    },
  };
}
