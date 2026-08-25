import { type Href, useRouter } from 'expo-router';

export interface BackOrReplaceNavigation {
  readonly canGoBack?: () => boolean;
  readonly back: () => void;
  readonly replace: (destination: Href) => void;
}

export function backOrReplace(
  navigation: BackOrReplaceNavigation,
  fallbackDestination: Href,
): void {
  if (navigation.canGoBack?.() === true) {
    navigation.back();
    return;
  }
  navigation.replace(fallbackDestination);
}

export function useBackOrReplace(): (fallbackDestination: Href) => void {
  const router = useRouter();
  return (fallbackDestination) => {
    backOrReplace(router, fallbackDestination);
  };
}
