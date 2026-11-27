# `@baukit/purchases-revenuecat`

The `react-native-purchases` implementation of the `@baukit/purchases-core` `PurchasesPort`. It
imports only types from `react-native-purchases`. The app passes the module in.

## Exports

| Export                                              | What it is                                                                  |
| --------------------------------------------------- | --------------------------------------------------------------------------- |
| `createRevenueCatPurchasesPort(purchases, options)` | Configures RevenueCat and returns a `PurchasesPort`.                        |
| `RevenueCatModule`                                  | The calls the adapter makes on the `react-native-purchases` default export. |
| `RevenueCatPurchasesOptions`                        | `{ apiKey }`, the public SDK key for the running platform.                  |

## Example

```ts
import Purchases from 'react-native-purchases';
import { createRevenueCatPurchasesPort } from '@baukit/purchases-revenuecat';

export async function startPurchases(apiKey: string, suiteSubject: string): Promise<void> {
  const port = createRevenueCatPurchasesPort(Purchases, { apiKey });
  await port.identify(suiteSubject);
  const offerings = await port.offerings();
  const first = offerings.current?.packages[0];
  if (first !== undefined) await port.purchase(first);
}
```

## Behavior

- The factory calls `Purchases.configure({ apiKey })` once and throws `unavailable` for an empty key. Use `createNoopPurchasesPort()` when the build has no key.
- `identify` calls `logIn` with the suite subject `<identity_domain>|<hub sub>`. The hub resolves RevenueCat webhooks against that string.
- `reset` calls `logOut` unless the user is already anonymous.
- `offerings` maps RevenueCat offerings. Offering and product IDs come from the RevenueCat dashboard and app configuration. `NON_SUBSCRIPTION` products become `one_time` packages for credit packs.
- RevenueCat error codes map to `cancelled`, `pending`, `network` or `store`.

`react-native-purchases` 10.14.0 or later is a peer dependency. It needs no Expo config plugin. Expo Go has no store access, so use a development build.

See the [suite and platform guide](../../../docs/guide/suite-and-platform.md) for the purchase
flow and `@baukit/purchases-core` for the port.
