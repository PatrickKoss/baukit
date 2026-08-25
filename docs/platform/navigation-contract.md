# Navigation contract

**Status:** Platform contract for generated web and mobile products

Contract only. Baukit ships the web reference implementation in
`templates/web/web/src/back-or-replace.ts`; products own their router.

## The problem

A screen reached by a deep link has no history behind it. The user opened a
push notification, followed a shared URL, or came back to a cold-started app.
Its close or back affordance calls the router's `back()`, the stack is empty,
the call is a no-op, and the button does nothing. The user is stuck on a screen
whose only exit silently fails.

This is not an edge case. Every deep-linkable screen hits it the first time
someone shares a link.

## The contract

Every screen that can be reached directly must resolve its back affordance
through a fallback:

1. If the router reports usable history, go back. The user returns where they
   came from, which is what they expect.
2. Otherwise, **replace** the current entry with a product-supplied fallback
   route. Replace, not push: the screen the user is leaving must not become the
   thing their next back press returns to.

The fallback is the screen's semantic parent, not a home route. A detail screen
falls back to its list, a nested form falls back to the flow that owns it. Give
each call site its own fallback rather than sharing one global default.

```ts
export interface BackOrReplaceNavigation {
  readonly canGoBack: () => boolean;
  readonly back: () => void;
  readonly replace: (destination: string) => void;
}

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
```

The parameter is a three-method shape, not a concrete router type. That is what
makes the rule testable with a plain object literal and portable between web and
native, where the router differs but the failure does not.

## Web and mobile

The rule is identical on both. Only the adapter changes.

- **Web:** `canGoBack` reads `history.length > 1`, `back` calls
  `history.back()`, `replace` calls `location.replace(destination)`. A router
  such as React Router supplies its own three operations; prefer them over the
  raw History API when one is present, because the router knows which entries
  belong to the app.
- **Native:** Expo Router's `Router` exposes `canGoBack()`, `back()`, and
  `replace()` directly and behaves the same on iOS and Android. `canGoBack` is
  optional on the shape so a router that does not expose it falls through to
  `replace`, which is the safe direction.

## Evidence

A unit test asserting all three branches is the cheapest coverage and should
exist for every product:

- history present: `back` is called once, `replace` is not called;
- history reported empty: `replace` is called with the fallback, `back` is not;
- `canGoBack` absent: same as reported empty.

The browser gate adds the behavioral half. `e2e/tests/qa-route-state.spec.ts`
loads each configured deep link cold, with no history, and requires a working
recovery control on screen. A no-op back button fails that spec.
